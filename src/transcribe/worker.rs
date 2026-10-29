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
    /// All parts done (joined text, language), or the first error.
    Done(Result<(String, Option<String>), String>),
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
///    `P <json string>`.
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
    let handle = super::parakeet::resolve_model_path(live_model)
        .map_err(|e| e.to_string())
        .and_then(|path| ParakeetUnifiedHandle::load(&path, None).map_err(|e| e.to_string()));
    let unified = handle.as_ref().map_err(|e| e.clone()).and_then(|handle| {
        ParakeetUnified::from_shared_with_streaming_config(handle, streaming_config)
            .map_err(|e| e.to_string())
    });
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
        let unified = first.take().or_else(|| {
            let h = handle.as_ref().ok()?;
            ParakeetUnified::from_shared_with_streaming_config(h, streaming_config).ok()
        });
        if !live_session(
            config,
            &mut out,
            &blocks_rx,
            unified,
            &handle,
            &streaming_config,
        )? {
            return Ok(());
        }
    }
}

/// One recording of the live worker: READY, audio until an empty block or
/// EOF, then the response. Returns whether another recording may follow
/// (false after EOF).
#[cfg(feature = "parakeet")]
fn live_session(
    config: &crate::config::Config,
    out: &mut io::StdoutLock<'_>,
    blocks_rx: &std::sync::mpsc::Receiver<Option<Vec<f32>>>,
    mut unified: Option<parakeet_rs::ParakeetUnified>,
    handle: &Result<parakeet_rs::ParakeetUnifiedHandle, String>,
    streaming_config: &parakeet_rs::UnifiedStreamingConfig,
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
    let (part_tx, part_rx) = mpsc::channel::<Vec<f32>>();
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
            for part in part_rx {
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
            ))));
        });
    });

    writeln!(out, "{}", READY_SIGNAL)?;
    out.flush()?;

    let mut samples: Vec<f32> = Vec::new();
    let mut splitter = PauseSplitter::new();
    let mut parts_sent = 0usize;
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
                let _ = part_tx.send(samples[range].to_vec());
                parts_sent += 1;
            }
        }
        while let Ok(msg) = msg_rx.try_recv() {
            match msg {
                EngineMsg::Part(audio, took) => progress.part_done(audio, took),
                EngineMsg::Done(r) => early_done = Some(r),
            }
        }
        let Some(u) = unified.as_mut() else { continue };
        match u.transcribe_chunk(&block) {
            Ok(delta) if !delta.is_empty() => {
                last_partial.push_str(&delta);
                let shown = last_partial.trim_start();
                let words = preview_words(u);
                writeln!(out, "{}{}", WORDS_PREFIX, serde_json::to_string(&words)?)?;
                writeln!(out, "{}{}", PARTIAL_PREFIX, serde_json::to_string(shown)?)?;
                out.flush()?;
                if let Some(log) = revlog.as_mut() {
                    log.preview(samples.len() as f32 / 16000.0, shown, words);
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[worker] Live preview failed, stopping it: {}", e);
                unified = None;
            }
        }
    }

    let secs = samples.len() as f32 / 16000.0;
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
                writeln!(
                    out,
                    "{}{}",
                    WORDS_PREFIX,
                    serde_json::to_string(&preview_words(u))?
                )?;
            }
            writeln!(
                out,
                "{}{}",
                PARTIAL_PREFIX,
                serde_json::to_string(last_partial.trim())?
            )?;
            out.flush()?;
            if let (Some(log), Some(u)) = (revlog.as_mut(), unified.as_ref()) {
                log.preview(
                    samples.len() as f32 / 16000.0,
                    last_partial.trim(),
                    preview_words(u),
                );
            }
        }
    }
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
            WorkerResponse::error("No speech recognized")
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
        let _ = part_tx.send(samples[range].to_vec());
    }
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
        Some(Ok((text, language))) => {
            eprintln!(
                "[worker] Final transcription in {:.2}s ({} parts sent while recording)",
                start.elapsed().as_secs_f32(),
                parts_sent
            );
            WorkerResponse::success(text, language)
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

/// The preview model's words so far, with their audio timing.
#[cfg(feature = "parakeet")]
fn preview_words(u: &parakeet_rs::ParakeetUnified) -> Vec<super::revisions::Word> {
    let timed = u.get_timed_transcript(parakeet_rs::TimestampMode::Tokens);
    let conf = u.token_confidences();
    super::revisions::words_from_tokens(
        timed
            .tokens
            .iter()
            .enumerate()
            .map(|(i, t)| (t.text.as_str(), t.start, t.end, conf.get(i).copied())),
    )
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
            .as_chunks::<4>().0
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
}
