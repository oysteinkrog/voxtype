//! Cohere Transcribe via the HuggingFace Optimum-exported ONNX format.
//!
//! Replaces the cstr-shaped implementation that was the proof-of-concept in
//! 0.7.0-rc1. The HF Optimum export is the upstream-canonical shape (used
//! by `onnx-community/cohere-transcribe-03-2026-ONNX`) and ships in four
//! precisions (FP16, int8, q4, q4f16). q4 is the one that compiles cleanly
//! through MIGraphX 7.2 and unblocks AMD GPU acceleration for Cohere.
//!
//! ## Architecture
//!
//! Two ONNX sessions:
//!
//! ### Encoder (`encoder_model.onnx`)
//! ```text
//! input_features  : F32 [batch, frames, 128]   # 128-bin log-mel + per-feature CMVN
//! ↓
//! last_hidden_state : F32 [batch, T_enc, 1024] # encoder output, T_enc downsampled
//! ```
//!
//! ### Decoder (`decoder_model_merged.onnx`)
//! ```text
//! input_ids                            : I64 [batch, seq]
//! attention_mask                       : I64 [batch, total_seq]   # cumulative
//! position_ids                         : I64 [batch, seq]
//! num_logits_to_keep                   : I64 []                   # scalar, always 1
//! encoder_hidden_states                : F32 [batch, T_enc, 1024]
//! past_key_values.{0..7}.decoder.{key,value} : F32 [batch, 8, past_dec, 128]
//! past_key_values.{0..7}.encoder.{key,value} : F32 [batch, 8, past_enc, 128]
//! ↓
//! logits                               : F32 [batch, num_logits_to_keep, 16384]
//! present.{0..7}.decoder.{key,value}   : F32 [batch, 8, total_dec, 128]
//! present.{0..7}.encoder.{key,value}   : F32 [batch, 8, T_enc, 128]
//! ```
//!
//! The decoder is a "merged" model: same graph handles both the prefix-fill
//! pass (empty past, multi-token input_ids) and incremental generation
//! (full past, single-token input_ids). The encoder's K/V projections are
//! computed lazily on the first decoder call (when `past.encoder` is empty)
//! and reused for every subsequent call via `past_key_values.N.encoder.*`,
//! so we don't re-project the encoder output every step.
//!
//! ## Decoder prefix
//!
//! Cohere uses a Whisper-style multi-token prompt to set language and task.
//! For English transcription with punctuation, ITN, no timestamps, no
//! diarization, the prefix is:
//!
//! ```text
//! [<|startoftranscript|>=4, <|en|>=62, <|pnc|>=5, <|itn|>=8,
//!  <|notimestamp|>=11, <|nodiarize|>=13]
//! ```

use crate::config::CohereConfig;
use crate::error::TranscribeError;
use crate::transcribe::cohere_fbank::CohereFbank;
use crate::transcribe::Transcriber;
use ort::session::Session;
use ort::value::{DynTensor, DynValue, Tensor, TensorElementType, ValueType};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tokenizers::Tokenizer;

// ---------------------------------------------------------------------------
// Architecture constants
// ---------------------------------------------------------------------------

const N_LAYERS: usize = 8;
const N_HEADS: usize = 8;
const HEAD_DIM: usize = 128;
const N_MELS: usize = 128;
const VOCAB_SIZE: usize = 16384;
const SAMPLE_RATE: usize = 16_000;

const TOK_EOS: i64 = 3;
const TOK_SOT: i64 = 4;
const TOK_PNC: i64 = 5;
const TOK_ITN: i64 = 8;
const TOK_NOTIMESTAMP: i64 = 11;
const TOK_NODIARIZE: i64 = 13;

fn lang_token(lang: &str) -> Option<i64> {
    Some(match lang {
        "ar" => 28,
        "de" => 76,
        "el" => 77,
        "en" => 62,
        "es" => 169,
        "fr" => 69,
        "it" => 97,
        "ja" => 98,
        "ko" => 110,
        "nl" => 60,
        "pl" => 148,
        "pt" => 149,
        "vi" => 194,
        "zh" => 50,
        _ => return None,
    })
}

const SUPPORTED_LANGUAGES: &[&str] = &[
    "ar", "de", "el", "en", "es", "fr", "it", "ja", "ko", "nl", "pl", "pt", "vi", "zh",
];

const MAX_TOKENS_PER_SECOND: f32 = 8.0;
const ABSOLUTE_MAX_TOKENS: usize = 1024;
/// The warm-up's clip length: the live worker's longest part
/// (`SPLIT_MAX_SECS` in the worker's pause splitter).
const WARM_UP_SECS: f32 = 30.0;

/// The encoder's TensorRT profile, in fbank frames (100 per second). The
/// engine's memory follows the longest input it takes, so it stops at 60 s
/// to fit an 8 GB card with a desktop running; longer audio is cut into
/// windows of at most this length (see `transcribe_windows`).
const TRT_MIN_FRAMES: usize = 1;
const TRT_OPT_FRAMES: usize = 1000;
const TRT_MAX_FRAMES: usize = 6000;
/// fbank frames per sample, rounded up: one frame every 10 ms.
const SAMPLES_PER_FRAME: usize = SAMPLE_RATE / 100;

// ---------------------------------------------------------------------------
// Transcriber
// ---------------------------------------------------------------------------

/// Per-step bundle of outputs the decoder loop pulls out of `SessionOutputs`
/// before the lock guard goes out of scope. Lets us release the decoder
/// mutex between steps.
struct StepOutputs {
    best_token: i64,
    present_dec: Vec<DynValue>,
    present_enc: Vec<DynValue>,
}

pub struct CohereTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tokenizer: Tokenizer,
    fbank: CohereFbank,
    prefix: Vec<i64>,
    /// Float dtype the encoder emits and the decoder expects on
    /// `encoder_hidden_states` and the `past_key_values.*` cache tensors.
    /// Float32 for the q4 / int8 / FP32 variants, Float16 for fp16 / q4f16.
    /// We discover this once at load time from the encoder's
    /// `last_hidden_state` output type so the decoder loop can build
    /// matching empty caches and we can extract the encoder output
    /// against the right primitive.
    float_dtype: TensorElementType,
    /// The encoder model, to load it again on CUDA if TensorRT fails.
    encoder_file: PathBuf,
    threads: usize,
    /// The encoder's TensorRT engine folder name.
    trt_key: String,
    /// The longest encoder input in frames while the encoder runs on
    /// TensorRT; 0 when it does not, and any length goes.
    encoder_max_frames: AtomicUsize,
}

impl CohereTranscriber {
    pub fn new(config: &CohereConfig) -> Result<Self, TranscribeError> {
        let model_dir = resolve_model_path(&config.model)?;
        let threads = config.threads.unwrap_or_else(|| num_cpus::get().min(4));
        Self::with_threads_and_lang(&model_dir, threads, &config.language)
    }

    pub fn from_dir(model_dir: &Path) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, num_cpus::get().min(4), "en")
    }

    pub fn with_threads(model_dir: &Path, threads: usize) -> Result<Self, TranscribeError> {
        Self::with_threads_and_lang(model_dir, threads, "en")
    }

    pub fn with_threads_and_lang(
        model_dir: &Path,
        threads: usize,
        language: &str,
    ) -> Result<Self, TranscribeError> {
        tracing::info!("Loading Cohere Transcribe model from {:?}", model_dir);
        let start = std::time::Instant::now();

        let encoder_file = model_dir.join("encoder_model.onnx");
        let decoder_file = model_dir.join("decoder_model_merged.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");

        for (label, path) in [
            ("encoder", &encoder_file),
            ("decoder", &decoder_file),
            ("tokenizer.json", &tokenizer_file),
        ] {
            if !path.exists() {
                let name = model_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                return Err(TranscribeError::ModelNotFound(format!(
                    "Cohere {label} not found: {}\n  Run: {}",
                    path.display(),
                    crate::model_catalog::download_command("cohere", &name),
                )));
            }
        }

        let tokenizer = Tokenizer::from_file(&tokenizer_file).map_err(|e| {
            TranscribeError::InitFailed(format!(
                "Failed to load Cohere tokenizer from {}: {e}",
                tokenizer_file.display()
            ))
        })?;

        let trt_key = encoder_cache_key(model_dir);
        let profile = encoder_profile();
        let (encoder, on_trt) = build_encoder(&encoder_file, threads, &trt_key, &profile)?;
        // Decoder pinned to CPU: ORT's CUDA GroupQueryAttention kernel rejects
        // the `attention_bias` input that the HF Optimum decoder export uses
        // (validated on GTX 1660 Ti, ORT 1.20 via pyke ort 2.0.0-rc.12). The
        // encoder still runs on GPU, where the 1.4GB-weight matmuls dominate
        // wall time; the smaller decoder runs CPU-side until ORT lands the
        // attention_bias kernel.
        // The decoder's threads spin: it runs once per token, and sleeping
        // between those small runs made the final pass 4 to 6 % slower.
        let decoder = build_session(&decoder_file, threads, "decoder", false, false, None)?;

        // The HF Optimum exports use mixed precision: `encoder_hidden_states`
        // stays Float32 across every variant (q4/int8/FP32 keep encoder
        // outputs in FP32; the FP16-flavored variants narrow inside the
        // decoder), but the KV caches and `logits` follow the variant —
        // Float32 for q4/int8/FP32, Float16 for fp16/q4f16. Detect the KV
        // dtype from the decoder's `past_key_values.0.decoder.key` input
        // so we build matching empty caches and read logits at the right
        // primitive size.
        let float_dtype = decoder
            .inputs()
            .iter()
            .find(|i| i.name() == "past_key_values.0.decoder.key")
            .and_then(|i| match i.dtype() {
                ValueType::Tensor { ty, .. } => Some(*ty),
                _ => None,
            })
            .unwrap_or(TensorElementType::Float32);
        if float_dtype != TensorElementType::Float32 && float_dtype != TensorElementType::Float16 {
            return Err(TranscribeError::InitFailed(format!(
                "Cohere decoder past_key_values dtype {:?} is neither Float32 nor Float16",
                float_dtype,
            )));
        }

        let lang_id = lang_token(language).ok_or_else(|| {
            TranscribeError::InitFailed(format!(
                "Unsupported Cohere language '{}'. Supported: {}",
                language,
                SUPPORTED_LANGUAGES.join(", "),
            ))
        })?;

        let prefix = vec![
            TOK_SOT,
            lang_id,
            TOK_PNC,
            TOK_ITN,
            TOK_NOTIMESTAMP,
            TOK_NODIARIZE,
        ];

        tracing::info!(
            "Cohere model loaded in {:.2}s (vocab={}, language='{}', prefix={:?})",
            start.elapsed().as_secs_f32(),
            VOCAB_SIZE,
            language,
            prefix,
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokenizer,
            fbank: CohereFbank::new(),
            prefix,
            float_dtype,
            encoder_file,
            threads,
            trt_key,
            encoder_max_frames: AtomicUsize::new(if on_trt { TRT_MAX_FRAMES } else { 0 }),
        })
    }

    /// The token sequences of `text` as the decoder could write it: as
    /// given, in lower case and with a capital first letter, each both
    /// after a space (the word marker the decoder writes) and without.
    fn token_variants(&self, text: &str) -> Vec<Vec<i64>> {
        let text = text.trim();
        let lower = text.to_lowercase();
        let capital: String = lower
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                let up: Vec<char> = if i == 0 {
                    c.to_uppercase().collect()
                } else {
                    vec![c]
                };
                up
            })
            .collect();
        let mut out: Vec<Vec<i64>> = Vec::new();
        let spelled = [text.to_string(), lower, capital];
        let spaced = spelled.iter().map(|v| format!(" {v}"));
        for variant in spelled.iter().cloned().chain(spaced) {
            if variant.trim().is_empty() {
                continue;
            }
            if let Ok(enc) = self.tokenizer.encode(variant.as_str(), false) {
                let ids: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
                if !ids.is_empty() && !out.contains(&ids) {
                    out.push(ids);
                }
            }
        }
        out
    }

    /// Transcribes audio longer than the TensorRT profile in equal windows
    /// that each fit it, and joins their text. A word on a window edge may
    /// be cut; this only runs for dictations over a minute.
    fn transcribe_windows(
        &self,
        samples: &[f32],
        avoid: &[Vec<i64>],
        push_down: f32,
        max_frames: usize,
    ) -> Result<String, TranscribeError> {
        let max_samples = (max_frames - 2) * SAMPLES_PER_FRAME;
        let windows = samples.len().div_ceil(max_samples);
        let len = samples.len().div_ceil(windows);
        tracing::info!(
            "Cohere: {:.1} s of audio is longer than the TensorRT profile, transcribing {windows} windows",
            samples.len() as f32 / SAMPLE_RATE as f32
        );
        let mut parts = Vec::with_capacity(windows);
        for window in samples.chunks(len) {
            let text = self.transcribe_samples(window, avoid, push_down)?;
            let text = text.trim();
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
        Ok(parts.join(" "))
    }

    fn transcribe_samples(
        &self,
        samples: &[f32],
        avoid: &[Vec<i64>],
        push_down: f32,
    ) -> Result<String, TranscribeError> {
        self.decode_samples(samples, avoid, push_down, false)
    }

    /// Run the encoder and the whole decoder loop once on the longest part
    /// the live worker sends ([`WARM_UP_SECS`]), ignoring EOS so the KV
    /// caches reach their largest size. The CUDA arena and cuDNN then
    /// already hold what a real part needs, so dictation does not allocate
    /// GPU memory or pick kernels.
    fn warm_up_run(&self) -> Result<(), TranscribeError> {
        // Quiet noise, not zeros: the fbank's log of an all-zero signal
        // is a constant, which is a valid input too, but noise is closer
        // to what the encoder sees.
        let n = (WARM_UP_SECS * SAMPLE_RATE as f32) as usize;
        let mut seed = 0x2545_f491_u32;
        let samples: Vec<f32> = (0..n)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32 - 0.5) * 0.002
            })
            .collect();
        self.decode_samples(&samples, &[], 0.0, true).map(|_| ())
    }

    /// The decoder runs until EOS, or with `full_length` for all
    /// `max_new` steps (warm-up only).
    fn decode_samples(
        &self,
        samples: &[f32],
        avoid: &[Vec<i64>],
        push_down: f32,
        full_length: bool,
    ) -> Result<String, TranscribeError> {
        // 1. Feature extraction.
        let max_frames = self.encoder_max_frames.load(Ordering::Relaxed);
        if max_frames > 0 && samples.len() / SAMPLES_PER_FRAME + 2 > max_frames {
            return self.transcribe_windows(samples, avoid, push_down, max_frames);
        }
        let features_2d = self.fbank.extract(samples);
        let n_frames = features_2d.nrows();
        if n_frames == 0 {
            return Ok(String::new());
        }
        let features_flat: Vec<f32> = features_2d.iter().copied().collect();
        let enc_input = Tensor::<f32>::from_array(([1usize, n_frames, N_MELS], features_flat))
            .map_err(|e| TranscribeError::InferenceFailed(format!("encoder input: {e}")))?;

        // 2. Encoder forward. We hold on to the encoder output as a
        // DynValue (rather than extracting it into a Vec<f32>) so the same
        // tensor flows back into the decoder by reference, and we don't
        // have to know whether it's Float32 or Float16 to copy it. The
        // decoder loop just borrows it by name each step.
        let encoder_hidden: DynValue = {
            let mut enc = self
                .encoder
                .lock()
                .map_err(|e| TranscribeError::InferenceFailed(format!("encoder lock: {e}")))?;
            let run = |enc: &mut Session, input: Tensor<f32>| {
                enc.run(ort::inputs!["input_features" => input])
                    .map(|mut out| out.remove("last_hidden_state"))
            };
            let enc_start = std::time::Instant::now();
            let hidden = match run(&mut enc, enc_input) {
                Ok(hidden) => hidden,
                Err(e) if self.encoder_max_frames.load(Ordering::Relaxed) > 0 => {
                    // TensorRT built the engine but could not run it. Move
                    // the encoder to CUDA for the life of this process.
                    tracing::warn!(
                        "Cohere encoder: TensorRT run failed ({e}); loading it on CUDA instead"
                    );
                    *enc = build_session(
                        &self.encoder_file,
                        self.threads,
                        "encoder",
                        true,
                        true,
                        None,
                    )?;
                    self.encoder_max_frames.store(0, Ordering::Relaxed);
                    super::onnx_ep::mark_tensorrt_failed(&self.trt_key, &e.to_string());
                    let features_flat: Vec<f32> = features_2d.iter().copied().collect();
                    let enc_input =
                        Tensor::<f32>::from_array(([1usize, n_frames, N_MELS], features_flat))
                            .map_err(|e| {
                                TranscribeError::InferenceFailed(format!("encoder input: {e}"))
                            })?;
                    run(&mut enc, enc_input).map_err(|e| {
                        TranscribeError::InferenceFailed(format!("encoder run: {e}"))
                    })?
                }
                Err(e) => {
                    return Err(TranscribeError::InferenceFailed(format!(
                        "encoder run: {e}"
                    )))
                }
            };
            tracing::debug!(
                "Cohere encoder: {n_frames} frames in {:.0} ms",
                enc_start.elapsed().as_secs_f64() * 1000.0
            );
            hidden.ok_or_else(|| {
                TranscribeError::InferenceFailed("encoder missing last_hidden_state".to_string())
            })?
        };

        // 3. Decoder generation loop.
        let audio_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        let max_new = ((audio_secs * MAX_TOKENS_PER_SECOND) as usize)
            .max(self.prefix.len() + 16)
            .min(ABSOLUTE_MAX_TOKENS);

        let mut generated: Vec<i64> = Vec::with_capacity(max_new);
        let mut tokens_so_far: Vec<i64> = self.prefix.clone();

        // KV caches grow each step (decoder) or stay constant after step 0
        // (encoder). On step 0 we pass empty caches with shape [1, 8, 0, 128]
        // in whatever float dtype the model uses (Float32 for q4/int8/FP32
        // exports, Float16 for fp16/q4f16).
        let mut past_dec: Vec<DynValue> = (0..N_LAYERS * 2)
            .map(|_| empty_kv(self.float_dtype))
            .collect::<Result<Vec<_>, _>>()?;
        let mut past_enc: Vec<DynValue> = (0..N_LAYERS * 2)
            .map(|_| empty_kv(self.float_dtype))
            .collect::<Result<Vec<_>, _>>()?;

        for step in 0..max_new {
            let (input_ids_vec, position_ids_vec, attention_mask_vec): (
                Vec<i64>,
                Vec<i64>,
                Vec<i64>,
            ) = if step == 0 {
                let pos: Vec<i64> = (0..self.prefix.len() as i64).collect();
                let mask: Vec<i64> = vec![1; self.prefix.len()];
                (self.prefix.clone(), pos, mask)
            } else {
                let last = *tokens_so_far.last().unwrap();
                let pos = vec![tokens_so_far.len() as i64 - 1];
                let mask: Vec<i64> = vec![1; tokens_so_far.len()];
                (vec![last], pos, mask)
            };

            let seq_len = input_ids_vec.len();
            let total_len = if step == 0 {
                self.prefix.len()
            } else {
                tokens_so_far.len()
            };

            let input_ids = Tensor::<i64>::from_array(([1usize, seq_len], input_ids_vec))
                .map_err(|e| TranscribeError::InferenceFailed(format!("input_ids tensor: {e}")))?;
            let attention_mask =
                Tensor::<i64>::from_array(([1usize, total_len], attention_mask_vec)).map_err(
                    |e| TranscribeError::InferenceFailed(format!("attention_mask tensor: {e}")),
                )?;
            let position_ids = Tensor::<i64>::from_array(([1usize, seq_len], position_ids_vec))
                .map_err(|e| {
                    TranscribeError::InferenceFailed(format!("position_ids tensor: {e}"))
                })?;
            // Scalar (rank-0) tensor — the inspector showed
            // `num_logits_to_keep : Tensor<Int64>[]`. A 1-D `[1]` shape
            // breaks the lm_head Slice op which uses this as a Starts arg.
            let num_logits = Tensor::<i64>::from_array(([] as [usize; 0], vec![1_i64]))
                .map_err(|e| TranscribeError::InferenceFailed(format!("num_logits tensor: {e}")))?;

            let mut inputs: Vec<(Cow<str>, ort::session::SessionInputValue)> = vec![
                (Cow::Borrowed("input_ids"), input_ids.into()),
                (Cow::Borrowed("attention_mask"), attention_mask.into()),
                (Cow::Borrowed("position_ids"), position_ids.into()),
                (Cow::Borrowed("num_logits_to_keep"), num_logits.into()),
                (
                    Cow::Borrowed("encoder_hidden_states"),
                    ort::session::SessionInputValue::from(&encoder_hidden),
                ),
            ];

            for layer in 0..N_LAYERS {
                let dk_name = format!("past_key_values.{layer}.decoder.key");
                let dv_name = format!("past_key_values.{layer}.decoder.value");
                let ek_name = format!("past_key_values.{layer}.encoder.key");
                let ev_name = format!("past_key_values.{layer}.encoder.value");
                inputs.push((
                    Cow::Owned(dk_name),
                    ort::session::SessionInputValue::from(&past_dec[layer * 2]),
                ));
                inputs.push((
                    Cow::Owned(dv_name),
                    ort::session::SessionInputValue::from(&past_dec[layer * 2 + 1]),
                ));
                inputs.push((
                    Cow::Owned(ek_name),
                    ort::session::SessionInputValue::from(&past_enc[layer * 2]),
                ));
                inputs.push((
                    Cow::Owned(ev_name),
                    ort::session::SessionInputValue::from(&past_enc[layer * 2 + 1]),
                ));
            }

            // Run the decoder, extract everything we need, then drop the
            // lock guard before starting the next iteration.
            let StepOutputs {
                best_token,
                present_dec,
                present_enc,
            } = {
                let mut dec = self
                    .decoder
                    .lock()
                    .map_err(|e| TranscribeError::InferenceFailed(format!("decoder lock: {e}")))?;
                let mut outputs = dec.run(inputs).map_err(|e| {
                    TranscribeError::InferenceFailed(format!("decoder step {step}: {e}"))
                })?;

                let pushed = pushed_down(&generated, avoid);
                let best = argmax_logits(&outputs["logits"], self.float_dtype, &pushed, push_down)?;

                let mut present_dec: Vec<DynValue> = Vec::with_capacity(N_LAYERS * 2);
                let mut present_enc: Vec<DynValue> = Vec::with_capacity(N_LAYERS * 2);
                for layer in 0..N_LAYERS {
                    let dk = outputs
                        .remove(format!("present.{layer}.decoder.key"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.decoder.key"
                            ))
                        })?;
                    let dv = outputs
                        .remove(format!("present.{layer}.decoder.value"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.decoder.value"
                            ))
                        })?;
                    let ek = outputs
                        .remove(format!("present.{layer}.encoder.key"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.encoder.key"
                            ))
                        })?;
                    let ev = outputs
                        .remove(format!("present.{layer}.encoder.value"))
                        .ok_or_else(|| {
                            TranscribeError::InferenceFailed(format!(
                                "missing present.{layer}.encoder.value"
                            ))
                        })?;
                    present_dec.push(dk);
                    present_dec.push(dv);
                    present_enc.push(ek);
                    present_enc.push(ev);
                }

                StepOutputs {
                    best_token: best,
                    present_dec,
                    present_enc,
                }
            };

            if best_token == TOK_EOS && !full_length {
                break;
            }
            generated.push(best_token);
            tokens_so_far.push(best_token);
            past_dec = present_dec;
            past_enc = present_enc;
        }

        let ids_u32: Vec<u32> = generated.iter().map(|&t| t as u32).collect();
        let text = self
            .tokenizer
            .decode(&ids_u32, true)
            .map_err(|e| TranscribeError::InferenceFailed(format!("tokenizer decode: {e}")))?;
        Ok(text.trim().to_string())
    }
}

/// Empty `[1, 8, 0, 128]` KV tensor in `dtype` (Float32 or Float16) for
/// the first decoder call. ort's raw-data constructors reject zero-sized
/// dimensions, so we go through `DynTensor::new(allocator, dtype, shape)`
/// which performs an empty allocation in the requested precision.
fn empty_kv(dtype: TensorElementType) -> Result<DynValue, TranscribeError> {
    let allocator = ort::memory::Allocator::default();
    let t = DynTensor::new(&allocator, dtype, [1usize, N_HEADS, 0, HEAD_DIM])
        .map_err(|e| TranscribeError::InferenceFailed(format!("empty kv: {e}")))?;
    Ok(t.into_dyn())
}

/// How far a pushed-down token's logit is lowered: about a 50th of its
/// probability, so a clearly heard word still wins.
const PUSH_DOWN_LOGIT: f32 = 4.0;

/// The tokens to push down after `generated`: for each sequence to avoid,
/// the token that would continue the longest part of it `generated` ends
/// with. Its first token is always pushed down, so the sequence cannot
/// start anywhere.
fn pushed_down(generated: &[i64], avoid: &[Vec<i64>]) -> Vec<i64> {
    avoid
        .iter()
        .filter_map(|seq| {
            (0..seq.len())
                .rev()
                .find(|&k| generated.ends_with(&seq[..k]))
                .map(|k| seq[k])
        })
        .collect()
}

/// Argmax over the last logit row, working in either Float32 or Float16
/// per the model variant, with the `pushed` tokens' logits lowered by
/// `by`. f16 values are widened to f32 for the comparison.
fn argmax_logits(
    logits: &DynValue,
    dtype: TensorElementType,
    pushed: &[i64],
    by: f32,
) -> Result<i64, TranscribeError> {
    let pick = |it: &mut dyn Iterator<Item = f32>| -> i64 {
        let mut best_idx = 0_i64;
        let mut best_val: Option<f32> = None;
        for (i, v) in it.enumerate() {
            let v = if pushed.contains(&(i as i64)) {
                v - by
            } else {
                v
            };
            if best_val.is_none_or(|b| v > b) {
                best_val = Some(v);
                best_idx = i as i64;
            }
        }
        best_idx
    };

    match dtype {
        TensorElementType::Float32 => {
            let (shape, data) = logits
                .try_extract_tensor::<f32>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("logits f32: {e}")))?;
            check_logits_shape(shape)?;
            Ok(pick(&mut data.iter().copied()))
        }
        TensorElementType::Float16 => {
            let (shape, data) = logits
                .try_extract_tensor::<half::f16>()
                .map_err(|e| TranscribeError::InferenceFailed(format!("logits f16: {e}")))?;
            check_logits_shape(shape)?;
            // Widen to f32 for the comparison; the relative ordering is
            // the same and we already pay one allocation worth of work.
            Ok(pick(&mut data.iter().map(|h| h.to_f32())))
        }
        other => Err(TranscribeError::InferenceFailed(format!(
            "unsupported logits dtype {:?}",
            other
        ))),
    }
}

fn check_logits_shape(shape: &[i64]) -> Result<(), TranscribeError> {
    if shape.len() != 3 || shape[2] as usize != VOCAB_SIZE {
        return Err(TranscribeError::InferenceFailed(format!(
            "logits shape {:?} != [1, 1, {VOCAB_SIZE}]",
            shape
        )));
    }
    Ok(())
}

impl Transcriber for CohereTranscriber {
    fn warm_up(&self) -> Result<(), TranscribeError> {
        self.warm_up_run()
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        let start = std::time::Instant::now();
        let text = self.transcribe_samples(samples, &[], 0.0)?;
        tracing::info!(
            "Cohere transcription completed in {:.2}s: {:?}",
            start.elapsed().as_secs_f32(),
            text.chars().take(80).collect::<String>(),
        );
        Ok(text)
    }

    fn transcribe_avoiding(&self, samples: &[f32], avoid: &str) -> Result<String, TranscribeError> {
        let avoid = self.token_variants(avoid);
        let text = self.transcribe_samples(samples, &avoid, PUSH_DOWN_LOGIT)?;
        tracing::info!(
            "Cohere correction clip, avoiding {} spellings: {:?}",
            avoid.len(),
            text
        );
        Ok(text)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_model_path(name: &str) -> Result<PathBuf, TranscribeError> {
    if let Ok(env) = std::env::var("VOXTYPE_COHERE_MODEL_DIR") {
        return Ok(PathBuf::from(env));
    }
    Ok(crate::config::Config::models_dir().join(name))
}

/// The encoder's TensorRT engine folder name: the model folder plus a hash
/// of the encoder files' sizes and times, because ORT names engine files
/// after the graph's structure and a new download of the same export keeps
/// that structure.
fn encoder_cache_key(model_dir: &Path) -> String {
    let name = model_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cohere".into());
    let mut files: Vec<(String, u64, u64)> = std::fs::read_dir(model_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            if !file.starts_with("encoder_model") {
                return None;
            }
            let meta = e.metadata().ok()?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            Some((file, meta.len(), mtime))
        })
        .collect();
    files.sort();
    // FNV-1a: stable across Rust releases, unlike DefaultHasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{files:?}").bytes() {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{name}-{:08x}-encoder", h as u32)
}

fn encoder_profile() -> super::onnx_ep::TrtProfile {
    let shape = |t: usize| format!("input_features:1x{t}x{N_MELS}");
    super::onnx_ep::TrtProfile {
        min: shape(TRT_MIN_FRAMES),
        opt: shape(TRT_OPT_FRAMES),
        max: shape(TRT_MAX_FRAMES),
    }
}

/// Loads the encoder on TensorRT when it is in use, and on CUDA when it is
/// not or when TensorRT fails to build it. Returns whether it is on
/// TensorRT.
fn build_encoder(
    path: &Path,
    threads: usize,
    cache_key: &str,
    profile: &super::onnx_ep::TrtProfile,
) -> Result<(Session, bool), TranscribeError> {
    if !super::onnx_ep::tensorrt_available() {
        return Ok((
            build_session(path, threads, "encoder", true, true, None)?,
            false,
        ));
    }
    if let Some(why) = super::onnx_ep::tensorrt_failed_before(cache_key) {
        tracing::warn!("Cohere encoder: TensorRT failed for this model before: {why}; using CUDA");
        return Ok((
            build_session(path, threads, "encoder", true, true, None)?,
            false,
        ));
    }
    let trt = super::onnx_ep::TrtSession {
        cache_key,
        profile: Some(profile),
    };
    let dir = super::onnx_ep::tensorrt_engine_dir(cache_key);
    let engines_before = dir.as_deref().map_or(0, count_engines);
    let start = std::time::Instant::now();
    match build_session(path, threads, "encoder", true, true, Some(&trt)) {
        Ok(session) => {
            let engines = dir.as_deref().map_or(0, count_engines);
            tracing::info!(
                "Cohere encoder on TensorRT in {:.1} s: {engines} cached engines ({} new) in {}",
                start.elapsed().as_secs_f32(),
                engines.saturating_sub(engines_before),
                dir.as_deref()
                    .map_or_else(String::new, |d| d.display().to_string()),
            );
            Ok((session, true))
        }
        Err(e) => {
            tracing::warn!("Cohere encoder: TensorRT could not build it ({e}); using CUDA");
            super::onnx_ep::mark_tensorrt_failed(cache_key, &e.to_string());
            Ok((
                build_session(path, threads, "encoder", true, true, None)?,
                false,
            ))
        }
    }
}

fn count_engines(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".engine"))
        .count()
}

fn build_session(
    path: &Path,
    threads: usize,
    label: &str,
    use_gpu: bool,
    sleep: bool,
    trt: Option<&super::onnx_ep::TrtSession>,
) -> Result<Session, TranscribeError> {
    let builder = Session::builder()
        .map_err(|e| TranscribeError::InitFailed(format!("{label} builder: {e}")))?
        .with_intra_threads(threads)
        .map_err(|e| TranscribeError::InitFailed(format!("{label} threads: {e}")))?;
    let builder = if sleep {
        super::onnx_ep::sleeping_threads(builder)
            .map_err(|e| TranscribeError::InitFailed(format!("{label} threads: {e}")))?
    } else {
        builder
    };

    let mut builder = if use_gpu {
        super::onnx_ep::register_gpu_eps_with(builder, "Cohere", label, trt)
            .map_err(|e| TranscribeError::InitFailed(format!("{label} EPs: {e}")))?
    } else {
        builder
    };

    builder.commit_from_file(path).map_err(|e| {
        TranscribeError::InitFailed(format!(
            "Failed to load Cohere {label} from {:?}: {e}",
            path
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushed_down_continues_the_longest_started_sequence() {
        let avoid = vec![vec![10, 11, 12], vec![20]];
        assert_eq!(pushed_down(&[], &avoid), vec![10, 20]);
        assert_eq!(pushed_down(&[5, 10], &avoid), vec![11, 20]);
        assert_eq!(pushed_down(&[10, 11], &avoid), vec![12, 20]);
        // A finished sequence starts over: only its first token again.
        assert_eq!(pushed_down(&[10, 11, 12], &avoid), vec![10, 20]);
        assert_eq!(pushed_down(&[1, 2], &[]), Vec::<i64>::new());
    }

    fn fixture(name: &str) -> Vec<f32> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/vad")
            .join(name);
        let reader = hound::WavReader::open(&path).expect("fixture opens");
        let max = (1 << (reader.spec().bits_per_sample - 1)) as f32;
        reader
            .into_samples::<i32>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / max)
            .collect()
    }

    /// Needs the cohere-transcribe-q4f16 model in the models folder:
    /// `cargo test --features cohere -- --ignored pushing_down`.
    #[test]
    #[ignore]
    fn pushing_down_the_heard_words_changes_the_text() {
        let config = CohereConfig {
            model: "cohere-transcribe-q4f16".into(),
            ..CohereConfig::default()
        };
        let cohere = CohereTranscriber::new(&config).expect("model loads");
        for name in ["speech_hello.wav", "speech_long.wav"] {
            let samples = fixture(name);
            let plain = cohere.transcribe(&samples).unwrap();
            assert!(!plain.is_empty());
            let first = plain.split_whitespace().next().unwrap().to_string();
            let pushed = cohere.transcribe_avoiding(&samples, &first).unwrap();
            let firmly = cohere
                .transcribe_samples(&samples, &cohere.token_variants(&first), 30.0)
                .unwrap();
            eprintln!(
                "{name}: plain {plain:?}, {first:?} pushed down {pushed:?}, firmly {firmly:?}"
            );
            assert_ne!(pushed.to_lowercase(), plain.to_lowercase());
            // The decoder honours the push: firmly, the word never starts it.
            assert!(!firmly.starts_with(&first), "{firmly:?}");
        }
    }
}
