//! Transcription worker process for GPU isolation
//!
//! This module implements a subprocess that handles transcription in isolation.
//! When `gpu_isolation = true`, the daemon spawns this worker for each
//! transcription, ensuring the GPU is fully released after transcription
//! completes (the process exits, releasing all GPU resources).
//!
//! Protocol (eager mode - subprocess spawned when recording starts):
//! 1. Worker starts, loads model
//! 2. Worker writes "READY\n" to stdout (signals model is loaded)
//! 3. Parent sends audio via stdin: [u32 sample_count (LE)][f32 samples (LE)...]
//! 4. Worker transcribes and writes JSON response to stdout
//! 5. Worker exits
//!
//! The key benefit: model loading happens while the user is speaking,
//! so perceived latency is just the transcription time.

use crate::config::WhisperConfig;
use crate::transcribe::Transcriber;
use std::io::{self, Read, Write};
use ureq::serde_json;

/// Ready signal sent after model is loaded
pub const READY_SIGNAL: &str = "READY";

/// JSON response from the worker
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum WorkerResponse {
    Success {
        ok: bool,
        text: String,
        /// Two-letter language code chosen for this transcription, if the
        /// worker tracked one. The parent reads this so layout-aware output
        /// methods (eitype, dotool) can switch keyboard layouts. Older
        /// parents that ignore the field simply skip the hint.
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        /// The final engine's text of each spoken correction (live worker).
        #[serde(skip_serializing_if = "Vec::is_empty")]
        corrections: Vec<crate::osd::correction::FinalClip>,
    },
    Error {
        ok: bool,
        error: String,
    },
}

impl WorkerResponse {
    pub fn success(text: String, language: Option<String>) -> Self {
        WorkerResponse::Success {
            ok: true,
            text,
            language,
            corrections: Vec::new(),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        WorkerResponse::Error {
            ok: false,
            error: msg.into(),
        }
    }
}

/// Run the transcription worker
///
/// This is the main entry point called from `voxtype transcribe-worker`.
/// It loads the model FIRST, signals ready, then waits for audio.
pub fn run_worker(config: &WhisperConfig) -> anyhow::Result<()> {
    run_worker_with(&config.model, || {
        super::whisper::WhisperTranscriber::new(config).map(|t| Box::new(t) as Box<dyn Transcriber>)
    })
}

/// Run the transcription worker for the configured non-Whisper engine
/// (`engine_isolation = true`). The parent passes `--engine` and `--model`,
/// which are already applied to `config`.
pub fn run_engine_worker(config: &crate::config::Config) -> anyhow::Result<()> {
    let mut config = config.clone();
    // This process is the isolated one; don't spawn another worker from it.
    config.engine_isolation = false;
    run_worker_with(config.model_name(), || super::create_transcriber(&config))
}

fn run_worker_with(
    model: &str,
    load: impl FnOnce() -> Result<Box<dyn Transcriber>, crate::error::TranscribeError>,
) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    // Step 1: Load model first (while user is speaking)
    eprintln!("[worker] Loading model: {}", model);
    let load_start = std::time::Instant::now();

    let transcriber = match load() {
        Ok(t) => t,
        Err(e) => {
            // Write error and exit - parent will see no READY signal
            write_response_to(
                &mut stdout_lock,
                WorkerResponse::error(format!("Failed to load model: {}", e)),
            );
            return Ok(());
        }
    };

    eprintln!(
        "[worker] Model loaded in {:.2}s",
        load_start.elapsed().as_secs_f32()
    );

    // Step 2: Signal ready (model is loaded, waiting for audio)
    writeln!(stdout_lock, "{}", READY_SIGNAL)?;
    stdout_lock.flush()?;
    eprintln!("[worker] Signaled READY, waiting for audio...");

    // Step 3: Read audio from stdin
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    // Read sample count (u32 little-endian)
    let mut count_buf = [0u8; 4];
    if let Err(e) = stdin.read_exact(&mut count_buf) {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!("Failed to read sample count: {}", e)),
        );
        return Ok(());
    }
    let sample_count = u32::from_le_bytes(count_buf) as usize;

    // Validate sample count (prevent OOM from malformed input)
    // Max 10 minutes at 16kHz = 9,600,000 samples = ~38MB
    const MAX_SAMPLES: usize = 16000 * 60 * 10;
    if sample_count > MAX_SAMPLES {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!(
                "Sample count too large: {} (max {})",
                sample_count, MAX_SAMPLES
            )),
        );
        return Ok(());
    }

    if sample_count == 0 {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error("Empty audio buffer"),
        );
        return Ok(());
    }

    // Read samples (f32 little-endian)
    let mut samples = vec![0f32; sample_count];
    let samples_bytes = unsafe {
        std::slice::from_raw_parts_mut(
            samples.as_mut_ptr() as *mut u8,
            sample_count * std::mem::size_of::<f32>(),
        )
    };

    if let Err(e) = stdin.read_exact(samples_bytes) {
        write_response_to(
            &mut stdout_lock,
            WorkerResponse::error(format!("Failed to read audio samples: {}", e)),
        );
        return Ok(());
    }

    eprintln!(
        "[worker] Received {} samples ({:.2}s)",
        sample_count,
        sample_count as f32 / 16000.0
    );

    // Step 4: Transcribe
    eprintln!("[worker] Starting transcription...");
    let transcribe_start = std::time::Instant::now();
    let result = transcriber.transcribe(&samples);

    match result {
        Ok(text) => {
            eprintln!(
                "[worker] Transcription complete in {:.2}s: {} chars",
                transcribe_start.elapsed().as_secs_f32(),
                text.len()
            );
            // Capture the chosen language so the parent can hint output
            // methods (eitype --layout, dotool DOTOOL_XKB_LAYOUT) about
            // what keyboard layout to use. Field is omitted from the JSON
            // if no language was tracked.
            let language = transcriber.last_detected_language();
            write_response_to(&mut stdout_lock, WorkerResponse::success(text, language));
        }
        Err(e) => {
            eprintln!("[worker] Transcription failed: {}", e);
            write_response_to(&mut stdout_lock, WorkerResponse::error(e.to_string()));
        }
    }

    Ok(())
}

/// Prefix of a live-preview line: `P <json string>\n`, the running transcript.
pub const PARTIAL_PREFIX: &str = "P ";

/// Prefix of a word timing line, sent just before the partial line it
/// belongs to: `W [{"text": "hello", "start": 0.32, "end": 0.56, "conf": 0.9}]`.
pub const WORDS_PREFIX: &str = "W ";

/// Prefix of a status line sent after recording stops, while the engine
/// model finishes: `S {"text": "...", "fraction": 0.8}`. `fraction` is the
/// share of the recording's audio transcribed so far, or null while the
/// model is still loading.
pub const STATUS_PREFIX: &str = "S ";

/// Prefix of a model signals line, sent after each streaming step of the
/// live model while the OSD is on: `G <json>`, an
/// [`crate::osd::live::SignalsStep`]. It never carries text that gets typed.
pub const SIGNALS_PREFIX: &str = "G ";

/// Error text of a final response when the recording held no words, as when a
/// dictation is started and stopped again without speaking. The daemon treats
/// it as "no speech", not as a failure.
pub const NO_SPEECH: &str = "No speech recognized";

/// Error text from the engine thread when the GPU has too little free memory.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
const GPU_BUSY: &str = "GPU busy";

/// Free memory on the first GPU in MiB, from nvidia-smi. `None` if it
/// cannot be read, so a missing nvidia-smi never blocks the engine.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
fn free_vram_mib() -> Option<u64> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// What the engine thread reports to the live worker's main loop.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
enum EngineMsg {
    /// A part is done: seconds of audio, seconds it took.
    Part(f32, f32),
    /// All parts done (joined text, language, the correction clips' texts
    /// in order), or the first error.
    Done(Result<(String, Option<String>, Vec<String>), String>),
}

/// Work for the engine thread.
#[cfg(feature = "parakeet")]
enum Job {
    /// Part of the clip, for the final text.
    Part(Vec<f32>),
    /// A spoken correction's audio, with the words it replaces pushed down.
    Clip(Vec<f32>, String),
}

/// Sends the parts of the final text with the spoken corrections' audio cut
/// out, as the daemon lists the holds in `correction-windows.json`. A part
/// that reaches into a hold still held waits for the key to come up.
#[cfg(feature = "parakeet")]
#[derive(Default)]
struct PartCuts {
    waiting: Vec<std::ops::Range<usize>>,
    /// Each part sent, with the holds that were cut from it.
    sent: Vec<(std::ops::Range<usize>, SampleRanges)>,
}

/// Holds as `(start, end)` sample indices.
#[cfg(feature = "parakeet")]
type SampleRanges = Vec<(usize, usize)>;

#[cfg(feature = "parakeet")]
use crate::osd::correction::WindowSpec;

#[cfg(feature = "parakeet")]
impl PartCuts {
    /// The holds as sample ranges, a hold still held running to `end`, and
    /// where a hold still held began.
    fn holds(specs: &[WindowSpec], end: usize) -> (Vec<(usize, usize)>, Option<usize>) {
        let at = |t: f32| ((t.max(0.0) * 16000.0) as usize).min(end);
        let open_from = specs
            .iter()
            .filter(|w| w.t1.is_none())
            .map(|w| at(w.t0))
            .min();
        let ranges = specs
            .iter()
            .map(|w| (at(w.t0), w.t1.map_or(end, at)))
            .filter(|(a, b)| a < b)
            .collect();
        (ranges, open_from)
    }

    /// `range` of `samples` is a part: send it now, or later if a hold
    /// still held may reach into it. `specs`: the holds so far.
    fn offer(
        &mut self,
        range: std::ops::Range<usize>,
        samples: &[f32],
        specs: &[WindowSpec],
        tx: &std::sync::mpsc::Sender<Job>,
    ) {
        self.waiting.push(range);
        self.retry(samples, specs, tx, false);
    }

    /// Sends the waiting parts that no held hold reaches into; all of them
    /// when `last` (the recording ended).
    fn retry(
        &mut self,
        samples: &[f32],
        specs: &[WindowSpec],
        tx: &std::sync::mpsc::Sender<Job>,
        last: bool,
    ) {
        if self.waiting.is_empty() {
            return;
        }
        let (holds, open_from) = Self::holds(specs, samples.len());
        let mut keep = Vec::new();
        for range in std::mem::take(&mut self.waiting) {
            let blocked = !last && open_from.is_some_and(|from| range.end > from);
            if blocked {
                keep.push(range);
                continue;
            }
            let cuts: Vec<(usize, usize)> = holds
                .iter()
                .copied()
                .filter(|&(a, b)| a < range.end && b > range.start)
                .collect();
            let part: Vec<f32> = range
                .clone()
                .filter(|i| !cuts.iter().any(|&(a, b)| (a..b).contains(i)))
                .map(|i| samples[i])
                .collect();
            if !part.is_empty() {
                let _ = tx.send(Job::Part(part));
            }
            self.sent.push((range, cuts));
        }
        self.waiting = keep;
    }

    fn waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// Whether every part that reaches into `hold` had it cut out.
    fn cut(&self, hold: (usize, usize)) -> bool {
        self.sent
            .iter()
            .filter(|(r, _)| hold.0 < r.end && hold.1 > r.start)
            .all(|(r, cuts)| {
                cuts.iter()
                    .any(|&(a, b)| a <= hold.0.max(r.start) && b >= hold.1.min(r.end))
            })
    }
}

/// Clips shorter than this use the preview model's final text when the
/// engine model has not finished loading yet, instead of waiting for it.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
const LIVE_MIN_WAIT_SECS: f32 = 2.5;

/// Splits a recording at pauses, so the engine model can transcribe each
/// part while the user is still talking. Works on 20 ms frames; a frame is
/// quiet when its RMS is under 3x the quietest frames of the current part,
/// so it follows the mic level instead of a fixed threshold.
struct PauseSplitter {
    /// Sample index where the current part starts.
    start: usize,
    /// RMS of each whole frame since `start`.
    frames: Vec<f32>,
}

const SPLIT_FRAME: usize = 320; // 20 ms at 16 kHz
                                // Pauses of 0.5 s are common inside a sentence, and a part cut there loses
                                // the context the model uses for casing and punctuation.
const SPLIT_MIN_SECS: f32 = 6.0;
const SPLIT_MAX_SECS: f32 = 30.0;
const SPLIT_PAUSE_FRAMES: usize = 40; // 0.8 s
/// After this long, a shorter pause will do, so the part left over when
/// the recording stops stays short.
const SPLIT_LONG_SECS: f32 = 15.0;
const SPLIT_SHORT_PAUSE_FRAMES: usize = 25; // 0.5 s
const SPLIT_MIN_SPEECH_FRAMES: usize = 15; // 0.3 s

impl PauseSplitter {
    fn new() -> Self {
        Self {
            start: 0,
            frames: Vec::new(),
        }
    }

    /// Quiet threshold for the current part: 3x its 5th-percentile frame RMS.
    fn threshold(&self) -> f32 {
        if self.frames.is_empty() {
            return 1e-3;
        }
        let mut sorted = self.frames.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        (sorted[sorted.len() / 20] * 3.0).max(1e-3)
    }

    /// How long the audio has been quiet up to the last whole frame: the
    /// frames at the end of the current part under its quiet threshold.
    #[cfg(feature = "parakeet")] // only the parakeet live worker uses this
    fn quiet_secs(&self) -> f32 {
        let t = self.threshold();
        let quiet = self.frames.iter().rev().take_while(|&&r| r < t).count();
        (quiet * SPLIT_FRAME) as f32 / 16000.0
    }

    /// True if frames `[from, to)` of the current part hold enough speech
    /// to be worth transcribing. Silence alone makes some models invent text.
    fn has_speech(&self, from: usize, to: usize) -> bool {
        let t = self.threshold();
        self.frames[from..to]
            .iter()
            .filter(|&&r| r > t * 2.0)
            .count()
            >= SPLIT_MIN_SPEECH_FRAMES
    }

    /// Look at the samples added since the last call. Returns the ranges of
    /// finished parts: `(range, has_speech)`.
    fn push(&mut self, samples: &[f32]) -> Vec<(std::ops::Range<usize>, bool)> {
        let mut done = Vec::new();
        loop {
            let next = self.start + self.frames.len() * SPLIT_FRAME;
            if next + SPLIT_FRAME > samples.len() {
                break;
            }
            let f = &samples[next..next + SPLIT_FRAME];
            self.frames
                .push((f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32).sqrt());

            let n = self.frames.len();
            let secs = (n * SPLIT_FRAME) as f32 / 16000.0;
            let pause = if secs < SPLIT_LONG_SECS {
                SPLIT_PAUSE_FRAMES
            } else {
                SPLIT_SHORT_PAUSE_FRAMES
            };
            let cut = if secs >= SPLIT_MIN_SECS && n >= pause {
                let t = self.threshold();
                if self.frames[n - pause..].iter().all(|&r| r < t) {
                    Some(n - pause / 2)
                } else if secs >= SPLIT_MAX_SECS {
                    // No pause: cut at the quietest 100 ms of the last 3 s.
                    let from = n - 150;
                    let best = (from..n - 5)
                        .min_by(|&a, &b| {
                            let ea: f32 = self.frames[a..a + 5].iter().sum();
                            let eb: f32 = self.frames[b..b + 5].iter().sum();
                            ea.total_cmp(&eb)
                        })
                        .unwrap_or(n);
                    Some(best + 2)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(c) = cut {
                let speech = self.has_speech(0, c);
                let end = self.start + c * SPLIT_FRAME;
                done.push((self.start..end, speech));
                self.start = end;
                self.frames.drain(..c);
            }
        }
        done
    }

    /// The unfinished part at the end of the recording, if it has speech.
    fn tail(&self, total: usize) -> Option<std::ops::Range<usize>> {
        let n = self.frames.len();
        let speech = n > 0 && self.has_speech(0, n);
        (speech && total > self.start).then_some(self.start..total)
    }
}

/// The parts of a whole recording that the live worker sends to the engine
/// model: split at pauses, only parts with speech. For offline replays.
pub fn speech_parts(samples: &[f32]) -> Vec<std::ops::Range<usize>> {
    let mut splitter = PauseSplitter::new();
    let mut parts: Vec<_> = splitter
        .push(samples)
        .into_iter()
        .filter(|(_, speech)| *speech)
        .map(|(r, _)| r)
        .collect();
    parts.extend(splitter.tail(samples.len()));
    parts
}

/// How far the engine model has got through the parts sent to it.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
struct Progress {
    /// Seconds of audio in each part sent but not done, oldest first.
    pending: std::collections::VecDeque<f32>,
    done_audio: f32,
    done_took: f32,
}

#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
impl Default for Progress {
    fn default() -> Self {
        Self {
            pending: Default::default(),
            done_audio: 0.0,
            done_took: 0.0,
        }
    }
}

#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
impl Progress {
    fn part_done(&mut self, audio: f32, took: f32) {
        self.pending.pop_front();
        self.done_audio += audio;
        self.done_took += took;
    }

    /// JSON for a status line. The time estimate uses the speed measured on
    /// the parts done so far, or 0.1 s per second of audio before that.
    fn status(&self, loaded: bool) -> String {
        let left: f32 = self.pending.iter().sum();
        let (text, fraction, eta) = if !loaded {
            ("Finishing: loading the model".to_string(), None, None)
        } else {
            let rate = if self.done_audio > 0.0 {
                self.done_took / self.done_audio
            } else {
                0.1
            };
            let total = self.done_audio + left;
            (
                format!(
                    "Finishing: {:.0} s of audio left, about {:.1} s",
                    left,
                    left * rate
                ),
                Some(if total > 0.0 {
                    self.done_audio / total
                } else {
                    1.0
                }),
                Some(left * rate),
            )
        };
        serde_json::json!({
            "text": text,
            "fraction": fraction,
            "audio_left_s": left,
            "eta_s": eta,
        })
        .to_string()
    }
}

/// Run the live-preview worker (`live_preview_model`).
///
/// Protocol:
/// 1. Worker starts reading stdin at once and loads the streaming preview
///    model (Parakeet cache-aware, CPU), then writes "READY". The engine
///    model (e.g. Cohere) loads on a background thread meanwhile.
/// 2. Parent sends any number of `[u32 sample_count (LE)][f32 samples...]`
///    blocks while recording, then an empty block (count 0) or EOF.
/// 3. For each change in the running transcript the worker writes
///    `W <words>` and `P <json string>`. With the OSD on it also writes
///    `G <json>` after each streaming step (see [`SIGNALS_PREFIX`]).
/// 4. Meanwhile the engine model transcribes each part of the clip that
///    ends in a pause (see [`PauseSplitter`]). After EOF only the last part
///    is left; until it is done the worker writes `S <json>` status lines,
///    then the usual JSON response with all parts joined.
/// 5. After an empty block the worker writes "READY" again and waits for the
///    next recording with the preview model still loaded; after EOF it exits.
#[cfg(feature = "parakeet")]
pub fn run_live_worker(config: &crate::config::Config, live_model: &str) -> anyhow::Result<()> {
    use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, UnifiedStreamingConfig};
    use std::sync::mpsc;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Read stdin from the start, so the parent never blocks on a full pipe
    // while the models load.
    // None marks the end of one recording.
    let (blocks_tx, blocks_rx) = mpsc::channel::<Option<Vec<f32>>>();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Ok(Some(block)) = read_block(&mut stdin) {
            if blocks_tx
                .send((!block.is_empty()).then_some(block))
                .is_err()
            {
                break;
            }
        }
    });

    eprintln!("[worker] Loading live preview model: {}", live_model);
    let load_start = std::time::Instant::now();
    // [parakeet] streaming_* sets the window; absent keys keep parakeet-rs's
    // own sizes (5.6 s left + 0.56 s chunk + 0.56 s right).
    let parakeet = config.parakeet.clone().unwrap_or_default();
    for note in parakeet.streaming_rounding_notes() {
        eprintln!("[worker] [parakeet] {} (whole 0.08 s steps)", note);
    }
    let streaming_config = match parakeet.streaming_window() {
        Ok(w) => {
            eprintln!("[worker] Live preview window: {}", w);
            super::parakeet_streaming::unified_streaming_config(w)
        }
        Err(e) => {
            eprintln!("[worker] {}; using the default live preview window", e);
            UnifiedStreamingConfig::default()
        }
    };
    // None = CPU: the GPU is left to the engine model.
    let model_path = super::parakeet::resolve_model_path(live_model).map_err(|e| e.to_string());
    // The sound strip runs an encoder with one more output and a probe on
    // it; without it the worker loads the plain encoder as before.
    let probe = model_path
        .as_ref()
        .ok()
        .and_then(|path| sound_probe(config, path));
    let handle = model_path.clone().and_then(|path| {
        match &probe {
            Some(p) => ParakeetUnifiedHandle::load_with_layer(
                &path,
                Some((
                    &path.join(super::sound_probe::ENCODER_FILE),
                    p.output.as_str(),
                )),
                None,
            ),
            None => ParakeetUnifiedHandle::load(&path, None),
        }
        .map_err(|e| e.to_string())
    });
    // The model signals (word details, frame levels, the lookahead tail)
    // are for the OSD: without it the worker sends none, and skips the
    // lookahead decode (a few ms per step).
    let vocab = config
        .osd
        .enabled
        .then(|| model_path.as_ref().ok())
        .flatten()
        .and_then(|path| {
            parakeet_rs::SentencePieceVocab::from_file(path.join("tokenizer.model"))
                .map_err(|e| eprintln!("[worker] No model signals: {}", e))
                .ok()
        });
    let new_unified = |handle: &ParakeetUnifiedHandle| {
        ParakeetUnified::from_shared_with_streaming_config(handle, streaming_config).map(|mut u| {
            u.set_tentative_enabled(vocab.is_some());
            u
        })
    };
    let unified = handle
        .as_ref()
        .map_err(|e| e.clone())
        .and_then(|handle| new_unified(handle).map_err(|e| e.to_string()));
    let mut first = match unified {
        Ok(u) => {
            eprintln!(
                "[worker] Live preview model loaded in {:.2}s",
                load_start.elapsed().as_secs_f32()
            );
            Some(u)
        }
        Err(e) => {
            // No preview, but the recording still gets transcribed.
            eprintln!("[worker] Live preview model failed to load: {}", e);
            None
        }
    };

    loop {
        // A fresh streaming state per recording; the weights stay loaded.
        let unified = first
            .take()
            .or_else(|| new_unified(handle.as_ref().ok()?).ok());
        if !live_session(
            config,
            &mut out,
            &blocks_rx,
            unified,
            &handle,
            &streaming_config,
            vocab.as_ref(),
            probe.as_ref(),
        )? {
            return Ok(());
        }
    }
}

/// One recording of the live worker: READY, audio until an empty block or
/// EOF, then the response. Returns whether another recording may follow
/// (false after EOF).
#[cfg(feature = "parakeet")]
#[allow(clippy::too_many_arguments)]
fn live_session(
    config: &crate::config::Config,
    out: &mut io::StdoutLock<'_>,
    blocks_rx: &std::sync::mpsc::Receiver<Option<Vec<f32>>>,
    mut unified: Option<parakeet_rs::ParakeetUnified>,
    handle: &Result<parakeet_rs::ParakeetUnifiedHandle, String>,
    streaming_config: &parakeet_rs::UnifiedStreamingConfig,
    vocab: Option<&parakeet_rs::SentencePieceVocab>,
    probe: Option<&super::sound_probe::SoundProbe>,
) -> anyhow::Result<bool> {
    use super::revisions::RevisionLog;
    use parakeet_rs::ParakeetUnified;
    use std::sync::mpsc;

    // Engine model in the background, started by the first audio block: its
    // load is hidden behind speech, and a worker kept warm between
    // recordings (live_preview_warm_secs) holds no VRAM while it waits.
    let mut engine_config = config.clone();
    engine_config.engine_isolation = false;
    let engine_model = engine_config.model_name().to_string();
    // Parts of the clip go in on part_tx; progress and then the joined
    // text (or the first error) come back on msg_rx.
    let (part_tx, part_rx) = mpsc::channel::<Job>();
    let (msg_tx, msg_rx) = mpsc::channel::<EngineMsg>();
    let engine_loaded = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loaded_flag = engine_loaded.clone();
    let min_free_vram = config.live_preview_min_free_vram_mib;
    let mut start_engine = (config.live_preview_final != "preview").then_some(move || {
        std::thread::spawn(move || {
            if let Some(free) = free_vram_mib().filter(|&f| f < min_free_vram) {
                let _ = msg_tx.send(EngineMsg::Done(Err(format!(
                    "{}: {} MiB free, {} needed",
                    GPU_BUSY, free, min_free_vram
                ))));
                return;
            }
            let start = std::time::Instant::now();
            let transcriber = match super::create_transcriber(&engine_config) {
                Ok(t) => t,
                Err(e) => {
                    let _ =
                        msg_tx.send(EngineMsg::Done(Err(format!("Failed to load model: {}", e))));
                    return;
                }
            };
            eprintln!(
                "[worker] {} loaded in {:.2}s",
                engine_model,
                start.elapsed().as_secs_f32()
            );
            loaded_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            let mut texts: Vec<String> = Vec::new();
            let mut clips: Vec<String> = Vec::new();
            for job in part_rx {
                let part = match job {
                    Job::Part(part) => part,
                    Job::Clip(clip, avoid) => {
                        // A clip that fails gives no correction text; the
                        // live words stand in for it.
                        let text = transcriber.transcribe_avoiding(&clip, &avoid);
                        if let Err(e) = &text {
                            eprintln!("[worker] Correction clip failed: {}", e);
                        }
                        clips.push(text.map(|t| t.trim().to_string()).unwrap_or_default());
                        continue;
                    }
                };
                let start = std::time::Instant::now();
                match transcriber.transcribe(&part) {
                    Ok(text) => {
                        let audio = part.len() as f32 / 16000.0;
                        let took = start.elapsed().as_secs_f32();
                        eprintln!("[worker] Part ({:.1}s of audio) in {:.2}s", audio, took);
                        let _ = msg_tx.send(EngineMsg::Part(audio, took));
                        let text = text.trim().to_string();
                        if !text.is_empty() {
                            texts.push(text);
                        }
                    }
                    Err(e) => {
                        let _ = msg_tx.send(EngineMsg::Done(Err(e.to_string())));
                        return;
                    }
                }
            }
            let _ = msg_tx.send(EngineMsg::Done(Ok((
                texts.join(" "),
                transcriber.last_detected_language(),
                clips,
            ))));
        });
    });

    writeln!(out, "{}", READY_SIGNAL)?;
    out.flush()?;

    let mut samples: Vec<f32> = Vec::new();
    let mut splitter = PauseSplitter::new();
    let mut parts_sent = 0usize;
    let mut cuts = PartCuts::default();
    let mut any_speech = false;
    let mut progress = Progress::default();
    let mut early_done = None;
    // transcribe_chunk() returns only the text new in that chunk (often
    // empty, sometimes part of a word), so the running transcript is the
    // concatenation.
    let mut last_partial = String::new();
    // Off unless live_preview_revisions_dir is set; the clock starts with
    // the first audio block.
    let mut revlog: Option<RevisionLog> = None;
    let mut signals = SignalsSent::new(probe);
    let mut ticks = TailTicks::new(
        config
            .parakeet
            .as_ref()
            .map_or(crate::config::DEFAULT_STREAMING_TAIL_SECS, |p| {
                p.streaming_tail_secs()
            }),
    );
    let mut pause = PauseCommit::new(
        config
            .parakeet
            .as_ref()
            .map_or(crate::config::DEFAULT_STREAMING_PAUSE_COMMIT_SECS, |p| {
                p.streaming_pause_commit_secs()
            }),
    );
    let mut more = false;
    for msg in blocks_rx.iter() {
        let Some(block) = msg else {
            more = true;
            break;
        };
        if let Some(start) = start_engine.take() {
            start();
        }
        if revlog.is_none() && config.live_preview_revisions_dir.is_some() {
            revlog = Some(RevisionLog::new());
        }
        samples.extend_from_slice(&block);
        for (range, speech) in splitter.push(&samples) {
            any_speech |= speech;
            if speech && config.live_preview_final != "preview" {
                // A send error means the engine failed to load; the error
                // is waiting on done_rx.
                progress.pending.push_back(range.len() as f32 / 16000.0);
                let specs = crate::osd::correction::read_windows();
                cuts.offer(range, &samples, &specs, &part_tx);
                parts_sent += 1;
            }
        }
        // A part waiting for a correction key to come up.
        if cuts.waiting() {
            let specs = crate::osd::correction::read_windows();
            cuts.retry(&samples, &specs, &part_tx, false);
        }
        while let Ok(msg) = msg_rx.try_recv() {
            match msg {
                EngineMsg::Part(audio, took) => progress.part_done(audio, took),
                EngineMsg::Done(r) => early_done = Some(r),
            }
        }
        let Some(u) = unified.as_mut() else { continue };
        match u.transcribe_chunk(&block) {
            Ok(delta) => {
                if !delta.is_empty() {
                    last_partial.push_str(&delta);
                    let shown = last_partial.trim_start();
                    let words = preview_words(u, vocab);
                    signals.note_words(&words, samples.len());
                    writeln!(out, "{}{}", WORDS_PREFIX, serde_json::to_string(&words)?)?;
                    if let Some(shown) = pause.committed(shown) {
                        writeln!(out, "{}{}", PARTIAL_PREFIX, serde_json::to_string(shown)?)?;
                    }
                    if let Some(log) = revlog.as_mut() {
                        log.preview(samples.len() as f32 / 16000.0, shown, words);
                    }
                }
                if let Some(vocab) = vocab {
                    signals.send(out, u, vocab, samples.len())?;
                    if ticks.due(u, samples.len()) {
                        let start = std::time::Instant::now();
                        match u.decode_tail() {
                            Ok(true) => {
                                ticks.done(samples.len(), start.elapsed());
                                signals.send_tail(out, u, vocab, samples.len())?;
                            }
                            Ok(false) => ticks.fresh_at = samples.len(),
                            Err(e) => {
                                eprintln!("[worker] Tail tick failed, stopping them: {}", e);
                                ticks.every = 0;
                            }
                        }
                    }
                    // A tail decoded in this block, over a long enough pause.
                    let tail_end = u
                        .tentative()
                        .last()
                        .map(|t| (t.abs_frame + 1) as f32 * LIVE_FRAME_SECS);
                    if let (true, Some(tail_end)) = (ticks.fresh_at == samples.len(), tail_end) {
                        let quiet = splitter.quiet_secs();
                        let with_tail = u.transcript_with_tentative();
                        if let Some(text) = pause.tail(
                            samples.len() as f32 / 16000.0,
                            quiet,
                            tail_end,
                            last_partial.trim(),
                            with_tail.trim(),
                        ) {
                            writeln!(out, "{}{}", PARTIAL_PREFIX, serde_json::to_string(&text)?)?;
                        }
                    }
                }
                out.flush()?;
            }
            Err(e) => {
                eprintln!("[worker] Live preview failed, stopping it: {}", e);
                unified = None;
            }
        }
    }

    let secs = samples.len() as f32 / 16000.0;
    if ticks.ticks > 0 {
        eprintln!(
            "[worker] Tail ticks: {} in {:.2}s of audio, {:.1} ms each",
            ticks.ticks,
            secs,
            ticks.took.as_secs_f32() * 1000.0 / ticks.ticks as f32
        );
    }
    // EOF between recordings: the daemon let the worker go.
    if samples.is_empty() && !more {
        return Ok(false);
    }
    eprintln!("[worker] Received {:.2}s of audio", secs);
    if samples.is_empty() {
        write_response_to(out, WorkerResponse::error("Empty audio buffer"));
        return Ok(more);
    }
    // flush() returns the text still held in the right context.
    if let Some(tail) = unified.as_mut().and_then(|u| u.flush().ok()) {
        if !tail.is_empty() {
            last_partial.push_str(&tail);
            // Show the last words too; the OSD stays up while finishing.
            if let Some(u) = unified.as_ref() {
                let words = preview_words(u, vocab);
                signals.note_words(&words, samples.len());
                writeln!(out, "{}{}", WORDS_PREFIX, serde_json::to_string(&words)?)?;
            }
            if let Some(shown) = pause.committed(last_partial.trim()) {
                writeln!(out, "{}{}", PARTIAL_PREFIX, serde_json::to_string(shown)?)?;
            }
            out.flush()?;
            if let (Some(log), Some(u)) = (revlog.as_mut(), unified.as_ref()) {
                log.preview(
                    samples.len() as f32 / 16000.0,
                    last_partial.trim(),
                    preview_words(u, vocab),
                );
            }
        }
    }
    if let (Some(u), Some(vocab)) = (unified.as_mut(), vocab) {
        signals.send(out, u, vocab, samples.len())?;
        out.flush()?;
    }
    signals.log_sounds();
    let mut preview_final = last_partial.trim().to_string();
    any_speech |= splitter.tail(samples.len()).is_some();
    // The streaming model has returned nothing for a whole clip with speech
    // in it (once, on a 30 s clip). Run it once more before giving up.
    if preview_final.is_empty() && any_speech {
        if let Ok(h) = handle.as_ref() {
            eprintln!(
                "[worker] Preview text empty despite speech; running the preview model again"
            );
            if let Ok(mut u) =
                ParakeetUnified::from_shared_with_streaming_config(h, *streaming_config)
            {
                u.set_tentative_enabled(false);
                let mut text = String::new();
                for chunk in samples.chunks(1600) {
                    if let Ok(d) = u.transcribe_chunk(chunk) {
                        text.push_str(&d);
                    }
                }
                if let Ok(tail) = u.flush() {
                    text.push_str(&tail);
                }
                preview_final = text.trim().to_string();
            }
        }
    }

    if config.live_preview_final == "preview" {
        let response = if preview_final.is_empty() {
            WorkerResponse::error(NO_SPEECH)
        } else {
            WorkerResponse::success(preview_final.clone(), None)
        };
        write_response_to(out, response);
        save_revisions(config, revlog, Some(&preview_final), "preview");
        save_to_corpus(
            config,
            &samples,
            Some(&preview_final),
            &preview_final,
            "preview",
        );
        return Ok(more);
    }

    // A short clip whose engine model is still loading gets the preview
    // model's text: waiting for the load would cost more than it gains.
    if parts_sent == 0
        && secs < LIVE_MIN_WAIT_SECS
        && !preview_final.is_empty()
        && !engine_loaded.load(std::sync::atomic::Ordering::SeqCst)
    {
        eprintln!("[worker] Short clip, model still loading: using preview text");
        write_response_to(out, WorkerResponse::success(preview_final.clone(), None));
        save_revisions(config, revlog, Some(&preview_final), "preview");
        save_to_corpus(
            config,
            &samples,
            Some(&preview_final),
            &preview_final,
            "preview",
        );
        return Ok(more);
    }
    if let Some(range) = splitter.tail(samples.len()) {
        progress.pending.push_back(range.len() as f32 / 16000.0);
        let specs = crate::osd::correction::read_windows();
        cuts.offer(range, &samples, &specs, &part_tx);
    }
    let specs = crate::osd::correction::read_windows();
    cuts.retry(&samples, &specs, &part_tx, true);
    // Each spoken correction's audio alone, with the live words it replaces
    // pushed down, so the final engine does not repeat the mistake.
    let clip_holds: Vec<crate::osd::correction::FinalClip> = {
        let end = samples.len() as f32 / 16000.0;
        let specs = crate::osd::correction::read_windows();
        let holds: Vec<(f32, f32)> = specs.iter().map(|w| (w.t0, w.t1.unwrap_or(end))).collect();
        let live: Vec<crate::osd::correction::Word> = unified
            .as_ref()
            .map(|u| preview_words(u, vocab))
            .unwrap_or_default()
            .into_iter()
            .map(|w| {
                let word = crate::osd::live::TimedWord {
                    text: w.text,
                    start: w.start,
                    end: w.end,
                    conf: w.conf,
                };
                (word, w.detail)
            })
            .collect();
        let wrong = crate::osd::correction::wrong_spans(live, &holds);
        holds
            .iter()
            .zip(wrong)
            .filter_map(|(&(t0, t1), wrong)| {
                let at = |t: f32| ((t.max(0.0) * 16000.0) as usize).min(samples.len());
                let (a, b) = (at(t0), at(t1));
                (a < b).then(|| {
                    let _ =
                        part_tx.send(Job::Clip(samples[a..b].to_vec(), wrong.unwrap_or_default()));
                    crate::osd::correction::FinalClip {
                        t0,
                        t1,
                        text: String::new(),
                        cut: cuts.cut((a, b)),
                    }
                })
            })
            .collect()
    };
    drop(part_tx);
    let start = std::time::Instant::now();
    let result = loop {
        if let Some(r) = early_done.take() {
            break Some(r);
        }
        let loaded = engine_loaded.load(std::sync::atomic::Ordering::SeqCst);
        // Nothing left once the last part is done; the text follows at once.
        if !loaded || !progress.pending.is_empty() {
            writeln!(out, "{}{}", STATUS_PREFIX, progress.status(loaded))?;
            out.flush()?;
        }
        match msg_rx.recv() {
            Ok(EngineMsg::Part(audio, took)) => progress.part_done(audio, took),
            Ok(EngineMsg::Done(r)) => break Some(r),
            Err(_) => break None,
        }
    };
    let response = match result {
        Some(Ok((text, language, clip_texts))) => {
            eprintln!(
                "[worker] Final transcription in {:.2}s ({} parts sent while recording)",
                start.elapsed().as_secs_f32(),
                parts_sent
            );
            let mut response = WorkerResponse::success(text, language);
            if let WorkerResponse::Success { corrections, .. } = &mut response {
                *corrections = clip_holds
                    .into_iter()
                    .zip(clip_texts)
                    .map(|(clip, text)| crate::osd::correction::FinalClip { text, ..clip })
                    .collect();
            }
            response
        }
        Some(Err(e)) if !preview_final.is_empty() => {
            eprintln!("[worker] Engine model failed, using preview: {}", e);
            if e.starts_with(GPU_BUSY) {
                let status = serde_json::json!({
                    "text": "GPU busy: pasting the live text",
                    "fraction": 1.0,
                });
                writeln!(out, "{}{}", STATUS_PREFIX, status)?;
                out.flush()?;
            }
            WorkerResponse::success(preview_final.clone(), None)
        }
        Some(Err(e)) => WorkerResponse::error(e),
        None if !preview_final.is_empty() => WorkerResponse::success(preview_final.clone(), None),
        None => WorkerResponse::error("Engine thread ended without a result"),
    };
    let final_text = match &response {
        WorkerResponse::Success { text, .. } => Some(text.clone()),
        _ => None,
    };
    write_response_to(out, response);
    // After the response, so it adds nothing to the wait.
    let source = if final_text.as_deref() == Some(preview_final.as_str()) {
        "preview"
    } else {
        "engine"
    };
    save_revisions(config, revlog, final_text.as_deref(), source);
    save_to_corpus(
        config,
        &samples,
        final_text.as_deref(),
        &preview_final,
        config.model_name(),
    );
    Ok(more)
}

/// The preview model's words so far, with their audio timing, and their
/// details when `vocab` is given (the OSD is on).
#[cfg(feature = "parakeet")]
fn preview_words(
    u: &parakeet_rs::ParakeetUnified,
    vocab: Option<&parakeet_rs::SentencePieceVocab>,
) -> Vec<super::revisions::Word> {
    let timed = u.get_timed_transcript(parakeet_rs::TimestampMode::Tokens);
    let conf = u.token_confidences();
    let details = u.token_details();
    super::revisions::words_with_details(timed.tokens.iter().enumerate().map(|(i, t)| {
        let detail = vocab.zip(details.get(i)).map(|(v, d)| word_detail(d, v));
        (
            t.text.as_str(),
            t.start,
            t.end,
            conf.get(i).copied(),
            detail,
        )
    }))
}

/// Seconds per encoder frame of the live model.
#[cfg(feature = "parakeet")]
const LIVE_FRAME_SECS: f32 = 0.08;

#[cfg(feature = "parakeet")]
fn word_detail(
    d: &parakeet_rs::TokenDetail,
    vocab: &parakeet_rs::SentencePieceVocab,
) -> crate::osd::live::WordDetail {
    // top[0] is the emitted token; blank as runner-up names no word.
    let (runner_up, _) = d.top[1];
    crate::osd::live::WordDetail {
        entropy: d.entropy,
        margin: d.margin,
        runner_up: (runner_up < vocab.size()).then(|| vocab.decode_single(runner_up)),
    }
}

/// Paces the tail-only decodes between streaming steps
/// (`[parakeet] streaming_tail_secs`): one each `every` samples of audio
/// since the tail was last new, from a step or a tick, unless the next step
/// is less than half an interval away. Stops for the rest of
/// the recording when two ticks in a row take more than
/// [`TAIL_TICK_BUDGET`] of their interval, so a slow CPU falls back to one
/// tail per step.
#[cfg(feature = "parakeet")]
struct TailTicks {
    every: usize,
    committed: usize,
    fresh_at: usize,
    slow: u32,
    ticks: u32,
    took: std::time::Duration,
}

/// The share of a tick's interval it may take.
#[cfg(feature = "parakeet")]
const TAIL_TICK_BUDGET: f32 = 0.6;

#[cfg(feature = "parakeet")]
impl TailTicks {
    fn new(secs: f32) -> Self {
        Self {
            every: (secs * 16000.0).round() as usize,
            committed: 0,
            fresh_at: 0,
            slow: 0,
            ticks: 0,
            took: std::time::Duration::ZERO,
        }
    }

    /// After a `transcribe_chunk`: whether to decode a new tail now.
    fn due(&mut self, u: &parakeet_rs::ParakeetUnified, samples: usize) -> bool {
        if u.committed_samples() != self.committed {
            // A step ran, and it decoded the tail.
            self.committed = u.committed_samples();
            self.fresh_at = samples;
            return false;
        }
        // A tick just before the next step would be replaced at once.
        let cfg = u.streaming_config();
        let step_at = self.committed + cfg.chunk_samples() + cfg.right_context_samples();
        self.every > 0
            && samples >= self.fresh_at + self.every
            && samples + self.every / 2 < step_at
    }

    /// Notes a tick that took `took`.
    fn done(&mut self, samples: usize, took: std::time::Duration) {
        self.fresh_at = samples;
        self.ticks += 1;
        self.took += took;
        let budget = TAIL_TICK_BUDGET * self.every as f32 / 16000.0;
        if took.as_secs_f32() > budget {
            self.slow += 1;
            if self.slow >= 2 {
                eprintln!(
                    "[worker] Tail ticks take {:.0} ms of {:.0} ms; one tail per step from now on",
                    took.as_secs_f32() * 1000.0,
                    self.every as f32 / 16.0
                );
                self.every = 0;
            }
        } else {
            self.slow = 0;
        }
    }
}

/// The live preview's text after a pause (`[parakeet]
/// streaming_pause_commit_secs`): once the audio has been quiet that long
/// and a tail was decoded over the quiet, the tail's words show as preview
/// text at once instead of waiting a step for their right context. The
/// committed text, and so the final text, do not change: while the steps
/// catch up with words already shown, their shorter text is not sent; text
/// that differs is sent, and the daemon shows it as a revision.
#[cfg(feature = "parakeet")]
struct PauseCommit {
    /// Seconds of quiet before the tail shows as text; 0 is off.
    after: f32,
    /// The preview text a pause sent, while the steps have not caught up.
    shown: Option<String>,
}

#[cfg(feature = "parakeet")]
impl PauseCommit {
    fn new(after: f32) -> Self {
        Self { after, shown: None }
    }

    /// The preview text to send for newly committed text `committed`, or
    /// `None` while the text a pause sent still begins with it or is it.
    fn committed<'a>(&mut self, committed: &'a str) -> Option<&'a str> {
        match &self.shown {
            Some(shown) if shown == committed => {
                self.shown = None;
                None
            }
            Some(shown) if shown.starts_with(committed) => None,
            _ => {
                self.shown = None;
                Some(committed)
            }
        }
    }

    /// After a tail decoded at `now` (seconds into the recording) over audio
    /// that has been quiet for `quiet_secs`: the committed text and the
    /// tail, `with_tail`, to send as preview text. Only when the pause is
    /// long enough both by the sound level and by the model, whose last tail
    /// token ended at `tail_end` (a guess the model keeps making up to the
    /// newest frame, as from noise, stays a guess), and it adds words to
    /// what shows.
    fn tail(
        &mut self,
        now: f32,
        quiet_secs: f32,
        tail_end: f32,
        committed: &str,
        with_tail: &str,
    ) -> Option<String> {
        let shows = self.shown.as_deref().unwrap_or(committed);
        if self.after <= 0.0
            || quiet_secs < self.after
            || now - tail_end < self.after
            || with_tail.len() <= shows.len()
            || !with_tail.starts_with(committed)
        {
            return None;
        }
        self.shown = Some(with_tail.to_string());
        self.shown.clone()
    }
}

/// The sound strip's probe, when the OSD is on and `sound-strip` is among
/// its motion features (`[osd] motion`, or `VOXTYPE_OSD_VO_MOTION` over it)
/// and both of its files are next to the live model
/// (`scripts/sound-probe-export.py` writes them).
#[cfg(feature = "parakeet")]
fn sound_probe(
    config: &crate::config::Config,
    model_dir: &std::path::Path,
) -> Option<super::sound_probe::SoundProbe> {
    use crate::osd::config::OsdMotion;
    if !config.osd.enabled {
        return None;
    }
    let env: Vec<String> = std::env::var("VOXTYPE_OSD_VO_MOTION")
        .ok()
        .into_iter()
        .collect();
    let motion = OsdMotion::layered(&env, &config.osd.motion);
    if !matches!(&motion, OsdMotion::Names(n) if n.iter().any(|n| n == "sound-strip")) {
        return None;
    }
    if !model_dir.join(super::sound_probe::ENCODER_FILE).exists() {
        eprintln!(
            "[worker] No sound strip: {} is not in {} (scripts/sound-probe-export.py makes it)",
            super::sound_probe::ENCODER_FILE,
            model_dir.display()
        );
        return None;
    }
    super::sound_probe::SoundProbe::load(&model_dir.join(super::sound_probe::PROBE_FILE))
        .map_err(|e| eprintln!("[worker] No sound strip: {}", e))
        .ok()
}

/// Sounds kept per frame of the sound strip.
#[cfg(feature = "parakeet")]
const SOUND_TOP_K: usize = 3;

/// What the worker has sent of the live model's signals this recording.
#[cfg(feature = "parakeet")]
#[derive(Default)]
struct SignalsSent<'a> {
    steps: u32,
    frames: usize,
    probe: Option<&'a super::sound_probe::SoundProbe>,
    sounds: SoundsSent,
}

/// What the sound strip has sent this recording, to log what it cost and
/// how far it ran ahead of the words.
#[cfg(feature = "parakeet")]
#[derive(Default)]
struct SoundsSent {
    runs: u32,
    took: std::time::Duration,
    /// Encoder frame -> samples heard when the strip first showed a speech
    /// sound there.
    heard: std::collections::BTreeMap<usize, usize>,
    /// Each word's start frame and the samples heard when it first showed.
    words: Vec<(usize, usize)>,
}

#[cfg(feature = "parakeet")]
impl SoundsSent {
    /// How far ahead of each word the strip showed a speech sound in the
    /// word's frames: those after the word before it, up to its own
    /// first frame (where the model emits it). Seconds, per word that had
    /// one.
    fn leads(&self) -> Vec<f32> {
        let mut from = 0usize;
        let mut leads = Vec::new();
        for &(frame, shown) in &self.words {
            // Two words can start on one frame: the second has none of its
            // own.
            if frame < from {
                continue;
            }
            let first = self.heard.range(from..=frame).map(|(_, &at)| at).min();
            if let Some(at) = first {
                leads.push(shown.saturating_sub(at) as f32 / 16000.0);
            }
            from = frame + 1;
        }
        leads
    }
}

#[cfg(feature = "parakeet")]
impl<'a> SignalsSent<'a> {
    fn new(probe: Option<&'a super::sound_probe::SoundProbe>) -> Self {
        Self {
            probe,
            ..Self::default()
        }
    }

    /// Notes when each word first showed, for the sound strip's lead.
    fn note_words(&mut self, words: &[super::revisions::Word], samples: usize) {
        if self.probe.is_none() {
            return;
        }
        for w in words.iter().skip(self.sounds.words.len()) {
            let frame = (w.start / LIVE_FRAME_SECS).round() as usize;
            self.sounds.words.push((frame, samples));
        }
    }

    /// The sounds of the frames committed since the last call and of the
    /// newest encoder run's lookahead frames.
    fn sounds(
        &mut self,
        u: &mut parakeet_rs::ParakeetUnified,
        samples: usize,
    ) -> (
        Vec<crate::osd::live::SoundFrame>,
        Vec<crate::osd::live::SoundFrame>,
    ) {
        use crate::osd::live::{Guess, SoundFrame};
        let Some(probe) = self.probe else {
            return Default::default();
        };
        let Some(layer) = u.take_layer_frames() else {
            return Default::default();
        };
        if layer.frames.ncols() != probe.dim() {
            return Default::default();
        }
        let start = std::time::Instant::now();
        let mut frames: Vec<SoundFrame> = Vec::with_capacity(layer.frames.nrows());
        for (i, row) in layer.frames.rows().into_iter().enumerate() {
            let abs = layer.first_frame + i;
            let row = row
                .to_slice()
                .map(<[f32]>::to_vec)
                .unwrap_or_else(|| row.to_vec());
            let top = probe.top(&row, SOUND_TOP_K);
            if top
                .first()
                .is_some_and(|&(c, _)| probe.labels[c] != super::sound_probe::SILENCE)
            {
                self.sounds.heard.entry(abs).or_insert(samples);
            }
            frames.push(SoundFrame {
                t: abs as f32 * LIVE_FRAME_SECS,
                top: top
                    .into_iter()
                    .map(|(c, logp)| Guess {
                        text: probe.labels[c].clone(),
                        logp,
                    })
                    .collect(),
            });
        }
        self.sounds.took += start.elapsed();
        self.sounds.runs += 1;
        let tail = frames.split_off(layer.committed.min(frames.len()));
        (frames, tail)
    }

    /// One stderr line per recording with the sound strip on: the probe's
    /// time per encoder run and how far the strip ran ahead of the words.
    fn log_sounds(&self) {
        let s = &self.sounds;
        if self.probe.is_none() || s.runs == 0 {
            return;
        }
        let mut leads = s.leads();
        leads.sort_by(f32::total_cmp);
        let ms = s.took.as_secs_f32() * 1000.0 / s.runs as f32;
        match (leads.first(), leads.last()) {
            (Some(min), Some(max)) => eprintln!(
                "[worker] Sound strip: probe {:.2} ms per step ({} steps); ahead of {} words \
                 by median {:.2}s (min {:.2}s, max {:.2}s)",
                ms,
                s.runs,
                leads.len(),
                leads[leads.len() / 2],
                min,
                max
            ),
            _ => eprintln!(
                "[worker] Sound strip: probe {:.2} ms per step ({} steps); no words",
                ms, s.runs
            ),
        }
    }
    /// Writes a `G` line when the model took a step since the last one:
    /// the frames new since then and the current lookahead tail.
    fn send(
        &mut self,
        out: &mut impl Write,
        u: &mut parakeet_rs::ParakeetUnified,
        vocab: &parakeet_rs::SentencePieceVocab,
        samples: usize,
    ) -> io::Result<()> {
        if u.frame_signals().len() <= self.frames {
            return Ok(());
        }
        self.write(out, u, vocab, samples)
    }

    /// Writes a `G` line for a tail tick ([`TailTicks`]): no new frames,
    /// the new lookahead tail.
    fn send_tail(
        &mut self,
        out: &mut impl Write,
        u: &mut parakeet_rs::ParakeetUnified,
        vocab: &parakeet_rs::SentencePieceVocab,
        samples: usize,
    ) -> io::Result<()> {
        self.write(out, u, vocab, samples)
    }

    fn write(
        &mut self,
        out: &mut impl Write,
        u: &mut parakeet_rs::ParakeetUnified,
        vocab: &parakeet_rs::SentencePieceVocab,
        samples: usize,
    ) -> io::Result<()> {
        use crate::osd::live::{FrameLevel, Guess, SignalsStep, TailToken};
        let all = u.frame_signals();
        let frames = all[self.frames.min(all.len())..]
            .iter()
            .map(|f| FrameLevel {
                t: f.abs_frame as f32 * LIVE_FRAME_SECS,
                level: f.log10_nonblank,
                top: f
                    .top
                    .iter()
                    .map(|&(id, logp)| Guess {
                        text: vocab.decode_single(id),
                        logp,
                    })
                    .collect(),
            })
            .collect();
        self.frames = all.len();
        self.steps += 1;
        let tail = u
            .tentative()
            .iter()
            .map(|t| TailToken {
                text: t.text.clone(),
                start: t.abs_frame as f32 * LIVE_FRAME_SECS,
                end: (t.abs_frame + 1) as f32 * LIVE_FRAME_SECS,
                edge: t.frames_to_edge as u32,
                conf: t.confidence,
                detail: Some(word_detail(&t.detail, vocab)),
            })
            .collect();
        let (sounds, sound_tail) = self.sounds(u, samples);
        let step = SignalsStep {
            step: self.steps,
            audio_s: samples as f32 / 16000.0,
            frames,
            tail,
            with_tail: u.transcript_with_tentative().trim().to_string(),
            sounds,
            sound_tail,
        };
        writeln!(out, "{}{}", SIGNALS_PREFIX, serde_json::to_string(&step)?)
    }
}

/// Write the revision log to `live_preview_revisions_dir`, if set.
/// `source` is "preview" when the final text is the preview model's own.
#[cfg(feature = "parakeet")]
fn save_revisions(
    config: &crate::config::Config,
    log: Option<super::revisions::RevisionLog>,
    final_text: Option<&str>,
    source: &str,
) {
    let (Some(log), Some(dir)) = (log, config.live_preview_revisions_dir.as_deref()) else {
        return;
    };
    let now = chrono::Local::now();
    let stem = now.format("%Y%m%d-%H%M%S%.3f").to_string();
    let model = if source == "preview" {
        config.live_preview_model.as_deref().unwrap_or_default()
    } else {
        config.model_name()
    };
    let header = serde_json::json!({
        "source": "live",
        "time": now.to_rfc3339(),
        "preview_model": config.live_preview_model,
        "final_model": model,
    });
    let dir = super::revisions::expand_home(dir);
    match log.write(&dir, &stem, header, None, source, model, final_text) {
        Ok(path) => eprintln!("[worker] Saved revisions to {}", path.display()),
        Err(e) => eprintln!("[worker] Could not save revisions: {}", e),
    }
}

/// Save a recording and its texts to `live_preview_corpus_dir`, if set.
/// Failures are logged and otherwise ignored: the text is already out.
#[cfg(feature = "parakeet")]
fn save_to_corpus(
    config: &crate::config::Config,
    samples: &[f32],
    final_text: Option<&str>,
    preview_text: &str,
    final_model: &str,
) {
    let Some(dir) = config.live_preview_corpus_dir.as_deref() else {
        return;
    };
    let dir = super::revisions::expand_home(dir);
    let now = chrono::Local::now();
    let dir = dir.join(now.format("%Y-%m").to_string());
    let stem = dir.join(now.format("%Y%m%d-%H%M%S%.3f").to_string());
    let result = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(stem.with_extension("wav"), spec)?;
        for &x in samples {
            wav.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        wav.finalize()?;
        let meta = serde_json::json!({
            "time": now.to_rfc3339(),
            "seconds": samples.len() as f32 / 16000.0,
            "final_text": final_text,
            "final_model": final_model,
            "preview_text": preview_text,
            "preview_model": config.live_preview_model,
            "corrected_text": serde_json::Value::Null,
        });
        std::fs::write(
            stem.with_extension("json"),
            serde_json::to_string_pretty(&meta)?,
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => eprintln!("[worker] Saved recording to {}.wav", stem.display()),
        Err(e) => eprintln!("[worker] Could not save recording: {}", e),
    }
}

#[cfg(not(feature = "parakeet"))]
pub fn run_live_worker(_config: &crate::config::Config, _live_model: &str) -> anyhow::Result<()> {
    anyhow::bail!("live_preview_model needs voxtype built with the parakeet feature")
}

/// Read one `[u32 count][f32 samples]` block. `Ok(None)` at a clean EOF.
#[cfg(feature = "parakeet")] // only the parakeet live worker uses this
fn read_block<R: Read>(reader: &mut R) -> io::Result<Option<Vec<f32>>> {
    let mut count_buf = [0u8; 4];
    match reader.read_exact(&mut count_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let count = u32::from_le_bytes(count_buf) as usize;
    if count > 16000 * 60 * 10 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("block too large: {} samples", count),
        ));
    }
    let mut bytes = vec![0u8; count * 4];
    reader.read_exact(&mut bytes)?;
    Ok(Some(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect(),
    ))
}

/// Write a JSON response to the given writer
fn write_response_to<W: Write>(writer: &mut W, response: WorkerResponse) {
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = writeln!(writer, "{}", json);
        let _ = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` of fake speech: 150 ms bursts of a 200 Hz tone at `amp`
    /// with 50 ms gaps, like syllables, over faint noise.
    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * 16000.0) as usize)
            .map(|i| {
                let noise = ((i * 7919) % 101) as f32 / 101.0 * 0.002 - 0.001;
                let on = (i % 3200) < 2400;
                let a = if on { amp } else { 0.0 };
                a * (i as f32 * 200.0 * std::f32::consts::TAU / 16000.0).sin() + noise
            })
            .collect()
    }

    /// LAT-4: the quiet at the end of the audio, in seconds, across a
    /// pause and after speech starts again.
    #[cfg(feature = "parakeet")]
    #[test]
    fn pause_splitter_measures_the_quiet_at_the_end() {
        let mut audio = tone(2.0, 0.3);
        let mut sp = PauseSplitter::new();
        sp.push(&audio);
        assert!(sp.quiet_secs() < 0.06, "{}", sp.quiet_secs());
        audio.extend(tone(0.4, 0.0));
        sp.push(&audio);
        assert!((sp.quiet_secs() - 0.4).abs() < 0.06, "{}", sp.quiet_secs());
        audio.extend(tone(0.2, 0.3));
        sp.push(&audio);
        assert!(sp.quiet_secs() < 0.06, "{}", sp.quiet_secs());
    }

    /// LAT-4: after a pause the tail shows as text; the steps that catch up
    /// with it send nothing, and text that reads differently is sent.
    #[cfg(feature = "parakeet")]
    #[test]
    fn pause_commit_shows_the_tail_and_waits_for_the_steps() {
        let mut p = PauseCommit::new(0.3);
        // At 2.5 s; the tail's last token ended at 2.1 s.
        let tail = |p: &mut PauseCommit, quiet: f32, end: f32, with_tail: &str| {
            p.tail(2.5, quiet, end, "open the", with_tail)
        };
        // Not quiet long enough, or nothing to add.
        assert_eq!(tail(&mut p, 0.2, 2.1, "open the file"), None);
        assert_eq!(tail(&mut p, 0.4, 2.1, "open the"), None);
        // A tail that does not start with the committed text.
        assert_eq!(tail(&mut p, 0.4, 2.1, "opened a file"), None);
        // A tail token the model heard less than the pause ago, as from
        // noise alone.
        assert_eq!(tail(&mut p, 0.4, 2.3, "open the file"), None);
        assert_eq!(
            tail(&mut p, 0.4, 2.1, "open the file and"),
            Some("open the file and".into())
        );
        // A longer tail in the same pause shows too; a shorter one does not.
        assert_eq!(tail(&mut p, 0.6, 2.1, "open the file"), None);
        assert_eq!(
            tail(&mut p, 0.6, 2.1, "open the file and save"),
            Some("open the file and save".into())
        );
        // Steps catching up send nothing, nor does reaching it.
        assert_eq!(p.committed("open the file"), None);
        assert_eq!(p.committed("open the file and save"), None);
        assert_eq!(
            p.committed("open the file and save it"),
            Some("open the file and save it")
        );
        // A step that reads it differently is sent at once.
        p.tail(2.5, 0.4, 2.1, "open the", "open the file and");
        assert_eq!(p.committed("open a"), Some("open a"));
        // Off.
        let mut off = PauseCommit::new(0.0);
        assert_eq!(off.tail(2.5, 5.0, 2.1, "open", "open the file"), None);
    }

    #[test]
    fn test_pause_splitter_cuts_at_pause_after_min_length() {
        let mut audio = tone(6.0, 0.3);
        audio.extend(tone(1.0, 0.0));
        audio.extend(tone(3.0, 0.3));
        let mut sp = PauseSplitter::new();
        let mut parts = Vec::new();
        // Feed in 100 ms blocks, as the daemon does.
        for end in (1600..=audio.len()).step_by(1600) {
            parts.extend(sp.push(&audio[..end]));
        }
        assert_eq!(parts.len(), 1);
        let (range, speech) = &parts[0];
        assert!(*speech);
        // Cut inside the pause, between 6.0 s and 7.0 s.
        assert!(
            range.end > 6 * 16000 && range.end < 7 * 16000,
            "{:?}",
            range
        );
        assert_eq!(sp.tail(audio.len()), Some(range.end..audio.len()));
    }

    #[test]
    fn test_pause_splitter_ignores_short_pause_and_silence() {
        let mut audio = tone(2.0, 0.3);
        audio.extend(tone(1.0, 0.0));
        audio.extend(tone(1.5, 0.3));
        let mut sp = PauseSplitter::new();
        // Pause ends at 3 s, before the 6 s minimum: no cut.
        assert!(sp.push(&audio).is_empty());

        let silence = tone(3.0, 0.0);
        let mut sp = PauseSplitter::new();
        assert!(sp.push(&silence).is_empty());
        assert_eq!(sp.tail(silence.len()), None);
    }

    #[test]
    fn test_pause_splitter_forces_cut_without_pause() {
        let audio = tone(35.0, 0.3);
        let mut sp = PauseSplitter::new();
        let parts = sp.push(&audio);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].0.end <= (SPLIT_MAX_SECS * 16000.0) as usize);
    }

    #[test]
    fn test_worker_response_serialization() {
        let success = WorkerResponse::success("Hello world".to_string(), None);
        let json = serde_json::to_string(&success).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""text":"Hello world""#));
        // No language present: serialized form omits the field entirely.
        assert!(!json.contains(r#""language""#));

        let error = WorkerResponse::error("Something went wrong");
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains(r#""ok":false"#));
        assert!(json.contains(r#""error":"Something went wrong""#));
    }

    #[test]
    fn test_worker_response_serialization_with_language() {
        let success = WorkerResponse::success("Привет".to_string(), Some("ru".to_string()));
        let json = serde_json::to_string(&success).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""language":"ru""#));
    }

    #[test]
    fn test_ready_signal() {
        assert_eq!(READY_SIGNAL, "READY");
    }

    /// Writes the live model's word lists for a public fixture, step by
    /// step, to tests/fixtures/correction/, for tests/correction_replay.rs.
    /// Needs the parakeet-unified-en-0.6b-int8 model:
    /// `cargo test --features parakeet -- --ignored record_live_words`.
    #[cfg(feature = "parakeet")]
    #[test]
    #[ignore]
    fn record_live_words_of_a_public_fixture() {
        use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, UnifiedStreamingConfig};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let reader = hound::WavReader::open(root.join("tests/fixtures/vad/speech_long.wav"))
            .expect("fixture opens");
        let max = (1 << (reader.spec().bits_per_sample - 1)) as f32;
        let samples: Vec<f32> = reader
            .into_samples::<i32>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / max)
            .collect();
        let path = super::super::parakeet::resolve_model_path("parakeet-unified-en-0.6b-int8")
            .expect("model path");
        let handle = ParakeetUnifiedHandle::load(&path, None).expect("model loads");
        let mut u = ParakeetUnified::from_shared_with_streaming_config(
            &handle,
            UnifiedStreamingConfig::default(),
        )
        .expect("live model");
        let mut steps = Vec::new();
        for chunk in samples.chunks(1600) {
            if !u.transcribe_chunk(chunk).expect("chunk").is_empty() {
                steps.push(preview_words(&u, None));
            }
        }
        if !u.flush().expect("flush").is_empty() {
            steps.push(preview_words(&u, None));
        }
        let out = root.join("tests/fixtures/correction/speech_long.live.json");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, serde_json::to_string_pretty(&steps).unwrap()).unwrap();
        eprintln!("{} steps to {}", steps.len(), out.display());
    }

    /// What the sound strip costs per streaming step on a public fixture:
    /// the encoder with and without the extra layer output, and the probe.
    /// Needs the live model with the files scripts/sound-probe-export.py
    /// writes: `cargo test --features parakeet -- --ignored sound_strip_cost
    /// --nocapture`, in an optimised profile for real numbers.
    #[cfg(feature = "parakeet")]
    #[test]
    #[ignore]
    fn sound_strip_cost() {
        use super::super::sound_probe::{SoundProbe, ENCODER_FILE, PROBE_FILE};
        use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, UnifiedStreamingConfig};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let reader = hound::WavReader::open(root.join("tests/fixtures/vad/speech_long.wav"))
            .expect("fixture opens");
        let max = (1 << (reader.spec().bits_per_sample - 1)) as f32;
        let samples: Vec<f32> = reader
            .into_samples::<i32>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / max)
            .collect();
        let path = super::super::parakeet::resolve_model_path("parakeet-unified-en-0.6b-int8")
            .expect("model path");
        let probe = SoundProbe::load(&path.join(PROBE_FILE)).expect("probe loads");
        let encoder = path.join(ENCODER_FILE);
        let mut texts = Vec::new();
        for with_layer in [false, true, false, true] {
            let handle = if with_layer {
                ParakeetUnifiedHandle::load_with_layer(&path, Some((&encoder, &probe.output)), None)
            } else {
                ParakeetUnifiedHandle::load(&path, None)
            }
            .expect("model loads");
            let (mut enc, mut prb, mut steps) = (0.0f64, 0.0f64, 0u32);
            // Several passes, so the first runs' warm-up counts little.
            for _ in 0..5 {
                let mut u = ParakeetUnified::from_shared_with_streaming_config(
                    &handle,
                    UnifiedStreamingConfig::default(),
                )
                .expect("live model");
                for chunk in samples.chunks(1600) {
                    let before = u.committed_samples();
                    u.transcribe_chunk(chunk).expect("chunk");
                    if u.committed_samples() == before {
                        continue;
                    }
                    steps += 1;
                    enc += u.last_step_stats().encoder.as_secs_f64();
                    if let Some(l) = u.take_layer_frames() {
                        let start = std::time::Instant::now();
                        for row in l.frames.rows() {
                            let row = row.to_vec();
                            std::hint::black_box(probe.top(&row, SOUND_TOP_K));
                        }
                        prb += start.elapsed().as_secs_f64();
                    }
                }
                u.flush().expect("flush");
                texts.push((with_layer, u.get_transcript()));
            }
            eprintln!(
                "layer output {}: encoder {:.2} ms per step, probe {:.3} ms per step ({} steps)",
                with_layer,
                enc * 1000.0 / steps as f64,
                prb * 1000.0 / steps as f64,
                steps
            );
        }
        // The extra output leaves the text as it was.
        assert!(texts.windows(2).all(|w| w[0].1 == w[1].1), "{texts:?}");
    }

    #[cfg(feature = "parakeet")]
    fn sent_parts(rx: &std::sync::mpsc::Receiver<Job>) -> Vec<usize> {
        rx.try_iter()
            .map(|job| match job {
                Job::Part(p) => p.len(),
                Job::Clip(..) => panic!("no clips here"),
            })
            .collect()
    }

    /// A correction's audio stays out of the final text's parts, and a part
    /// that reaches into a hold still held waits for the key to come up.
    #[cfg(feature = "parakeet")]
    #[test]
    fn parts_leave_out_the_correction_holds() {
        let samples = vec![0.1f32; 16000 * 4];
        let (tx, rx) = std::sync::mpsc::channel();
        let mut cuts = PartCuts::default();
        let held = [WindowSpec { t0: 1.5, t1: None }];
        cuts.offer(0..16000, &samples, &held, &tx);
        // A part before the hold goes at once.
        assert_eq!(sent_parts(&rx), [16000]);
        cuts.offer(16000..32000, &samples, &held, &tx);
        // A part reaching into the hold waits.
        assert!(sent_parts(&rx).is_empty());
        let done = [WindowSpec {
            t0: 1.5,
            t1: Some(2.5),
        }];
        cuts.retry(&samples, &done, &tx, false);
        // It goes once the key is up, without the hold's half second.
        assert_eq!(sent_parts(&rx), [8000]);
        cuts.offer(32000..64000, &samples, &done, &tx);
        assert_eq!(sent_parts(&rx), [32000 - 8000]);
        assert!(cuts.cut((24000, 40000)));
        // A part sent before the hold was known kept its audio.
        assert!(!cuts.cut((8000, 12000)));
    }

    // M0.2: pausing the live model for a keyboard edit by sending silence
    // instead of audio. Needs the parakeet-unified-en-0.6b-int8 model:
    // `cargo test --features parakeet --lib -- --ignored silence_pad`.
    //
    // Progress past the pause point is `ParakeetUnified::committed_samples()`
    // reaching the pause sample: every word before it has been decoded. The
    // worker shows it today only as the newest `G` line frame
    // (`FrameLevel.t` + LIVE_FRAME_SECS), sent when the OSD is on.

    #[cfg(feature = "parakeet")]
    fn fixture_samples(name: &str) -> Vec<f32> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let reader = hound::WavReader::open(root.join("tests/fixtures/vad").join(name))
            .expect("fixture opens");
        let max = (1 << (reader.spec().bits_per_sample - 1)) as f32;
        reader
            .into_samples::<i32>()
            .filter_map(Result::ok)
            .map(|s| s as f32 / max)
            .collect()
    }

    #[cfg(feature = "parakeet")]
    fn live_model() -> (
        parakeet_rs::ParakeetUnifiedHandle,
        parakeet_rs::UnifiedStreamingConfig,
    ) {
        let path = super::super::parakeet::resolve_model_path("parakeet-unified-en-0.6b-int8")
            .expect("model path");
        let handle = parakeet_rs::ParakeetUnifiedHandle::load(&path, None).expect("model loads");
        (handle, parakeet_rs::UnifiedStreamingConfig::default())
    }

    #[cfg(feature = "parakeet")]
    fn fresh(
        handle: &parakeet_rs::ParakeetUnifiedHandle,
        cfg: parakeet_rs::UnifiedStreamingConfig,
    ) -> parakeet_rs::ParakeetUnified {
        parakeet_rs::ParakeetUnified::from_shared_with_streaming_config(handle, cfg)
            .expect("live model")
    }

    /// Feeds `samples` in 0.1 s blocks, as the daemon sends them.
    #[cfg(feature = "parakeet")]
    fn feed(u: &mut parakeet_rs::ParakeetUnified, samples: &[f32]) {
        for block in samples.chunks(1600) {
            u.transcribe_chunk(block).expect("chunk");
        }
    }

    #[cfg(feature = "parakeet")]
    fn plain(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }

    #[cfg(feature = "parakeet")]
    fn reference(
        handle: &parakeet_rs::ParakeetUnifiedHandle,
        cfg: parakeet_rs::UnifiedStreamingConfig,
        samples: &[f32],
    ) -> Vec<super::super::revisions::Word> {
        let mut u = fresh(handle, cfg);
        feed(&mut u, samples);
        u.flush().expect("flush");
        preview_words(&u, None)
    }

    /// Pause after word N with a silence pad of chunk + right context, then
    /// resume: the N words are all decoded before resume, and the words
    /// after it are neither skipped nor repeated, at their own times once
    /// the pad is taken off.
    #[cfg(feature = "parakeet")]
    #[test]
    #[ignore]
    fn silence_pad_pause_keeps_every_word() {
        let samples = fixture_samples("speech_long.wav");
        let (handle, cfg) = live_model();
        let all = reference(&handle, cfg, &samples);
        let pad_len = cfg.chunk_samples() + cfg.right_context_samples();
        assert_eq!(pad_len, 17920, "1.12 s at the default window");
        let mut paused_any = 0;
        for n in 1..all.len() {
            // A tap in the gap between word n-1 and word n.
            let gap = (all[n - 1].end, all[n].start);
            // A tap comes after the speaker stops. Token times lead the
            // audio, so a shorter gap is often inside a word; see
            // silence_pad_mid_word_pause_leaves_a_fragment.
            if gap.1 - gap.0 < 0.3 {
                continue;
            }
            paused_any += 1;
            let pause = ((gap.0 + gap.1) / 2.0 * 16000.0) as usize;
            let mut u = fresh(&handle, cfg);
            feed(&mut u, &samples[..pause]);
            assert!(u.committed_samples() < pause);
            let mut padded = 0;
            let mut progress_at = None;
            for block in vec![0.0f32; pad_len].chunks(1600) {
                u.transcribe_chunk(block).expect("pad");
                padded += block.len();
                if progress_at.is_none() && u.committed_samples() >= pause {
                    progress_at = Some(padded);
                }
            }
            let progress_at = progress_at.expect("the pad commits past the pause point");
            // The marker as the G line shows it.
            let newest = u.frame_signals().last().expect("frames").abs_frame;
            assert_eq!((newest + 1) * 1280, u.committed_samples());
            // Every word before the tap is decoded. Word times lead the
            // audio a little, so the word just after the gap can be there
            // too when its onset fell before the pause point.
            let before: Vec<_> = preview_words(&u, None)
                .iter()
                .map(|w| plain(&w.text))
                .collect();
            let want: Vec<_> = all.iter().map(|w| plain(&w.text)).collect();
            assert!(
                before.len() >= n && before[..] == want[..before.len()],
                "pause after word {n}: {before:?} is not the first {n} or more of {want:?}"
            );
            // Resume.
            feed(&mut u, &samples[pause..]);
            u.flush().expect("flush");
            let words = preview_words(&u, None);
            // Stream time to speech time. Words decoded before resume keep
            // their time (at most the pause point); later words move back
            // by the pad. Token times lead the audio by up to about 0.2 s,
            // so a word after resume can carry a time inside the pad: which
            // side of the pause a word is on comes from when it arrived,
            // not from its time. The first word after resume can even be
            // timed near the start of the pad ("with" at 2.96 s stream time
            // for a pause at 2.72 s), so it maps to the pause point, early
            // by up to the length of the gap.
            let (pause_s, pad_s) = (pause as f32 / 16000.0, pad_len as f32 / 16000.0);
            let shifted: Vec<(String, f32)> = words
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let t = if i < before.len() {
                        w.start.min(pause_s)
                    } else {
                        (w.start - pad_s).max(pause_s)
                    };
                    (plain(&w.text), t)
                })
                .collect();
            assert_eq!(
                shifted.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
                all.iter().map(|w| plain(&w.text)).collect::<Vec<_>>(),
                "pause after word {n}: no word skipped or repeated"
            );
            for (i, (w, r)) in shifted.iter().zip(&all).enumerate() {
                // The first word after resume is timed early, down to the
                // pause point, but never late.
                let ok = if i == before.len() {
                    w.1 >= pause_s && w.1 <= r.start + 0.08
                } else {
                    (w.1 - r.start).abs() <= 0.08
                };
                assert!(
                    ok,
                    "pause after word {n}: {:?} at {} not {}",
                    w.0, w.1, r.start
                );
            }
            eprintln!(
                "pause after word {n} at {:.2}s: committed past it after {:.2}s of pad",
                pause as f32 / 16000.0,
                progress_at as f32 / 16000.0
            );
        }
        assert!(paused_any >= 3, "the fixture has gaps to pause in");
    }

    /// Frames held while a tap is pending and then sent at once give the
    /// same words and times as frames sent as they came.
    #[cfg(feature = "parakeet")]
    #[test]
    #[ignore]
    fn silence_pad_held_frames_decode_as_unheld() {
        let samples = fixture_samples("speech_long.wav");
        let (handle, cfg) = live_model();
        let all = reference(&handle, cfg, &samples);
        for (at, held) in [(8000, 4800), (25000, 4800), (40000, 6400)] {
            let mut u = fresh(&handle, cfg);
            feed(&mut u, &samples[..at]);
            u.transcribe_chunk(&samples[at..at + held]).expect("held");
            feed(&mut u, &samples[at + held..]);
            u.flush().expect("flush");
            let words = preview_words(&u, None);
            let got: Vec<_> = words.iter().map(|w| (&w.text, w.start, w.end)).collect();
            let want: Vec<_> = all.iter().map(|w| (&w.text, w.start, w.end)).collect();
            assert_eq!(got, want, "held {held} samples at {at}");
        }
    }

    /// A pause inside a word (between "voice" and "activity", where the
    /// token gap is 0.16 s but the audio is not silent) decodes the start
    /// of the word before the pause and the whole word again after it.
    /// Pinned so a change in this behaviour is noticed.
    #[cfg(feature = "parakeet")]
    #[test]
    #[ignore]
    fn silence_pad_mid_word_pause_leaves_a_fragment() {
        let samples = fixture_samples("speech_long.wav");
        let (handle, cfg) = live_model();
        let pause = (1.60 * 16000.0) as usize;
        let mut u = fresh(&handle, cfg);
        feed(&mut u, &samples[..pause]);
        feed(
            &mut u,
            &vec![0.0f32; cfg.chunk_samples() + cfg.right_context_samples()],
        );
        feed(&mut u, &samples[pause..]);
        u.flush().expect("flush");
        let text: Vec<_> = preview_words(&u, None)
            .iter()
            .map(|w| plain(&w.text))
            .collect();
        assert_eq!(
            text,
            [
                "this",
                "is",
                "a",
                "longer",
                "test",
                "of",
                "voice",
                "act",
                "activity",
                "detection",
                "with",
                "multiple",
                "words",
                "and",
                "phrases"
            ]
        );
    }
}
