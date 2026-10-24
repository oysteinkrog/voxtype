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
        super::whisper::WhisperTranscriber::new(config)
            .map(|t| Box::new(t) as Box<dyn Transcriber>)
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

/// Clips shorter than this use the preview model's final text when the
/// engine model has not finished loading yet, instead of waiting for it.
const LIVE_MIN_WAIT_SECS: f32 = 2.5;

/// Run the live-preview worker (`live_preview_model`).
///
/// Protocol:
/// 1. Worker starts reading stdin at once and loads the streaming preview
///    model (Parakeet cache-aware, CPU), then writes "READY". The engine
///    model (e.g. Cohere) loads on a background thread meanwhile.
/// 2. Parent sends any number of `[u32 sample_count (LE)][f32 samples...]`
///    blocks while recording, then closes stdin.
/// 3. For each change in the running transcript the worker writes
///    `P <json string>`.
/// 4. After EOF the worker writes the usual JSON response with the engine
///    model's transcript of the whole clip and exits.
#[cfg(feature = "parakeet")]
pub fn run_live_worker(config: &crate::config::Config, live_model: &str) -> anyhow::Result<()> {
    use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, UnifiedStreamingConfig};
    use std::sync::mpsc;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Read stdin from the start, so the parent never blocks on a full pipe
    // while the models load.
    let (blocks_tx, blocks_rx) = mpsc::channel::<Vec<f32>>();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        while let Ok(Some(block)) = read_block(&mut stdin) {
            if blocks_tx.send(block).is_err() {
                break;
            }
        }
    });

    // Engine model in the background, started by the first audio block: its
    // load is hidden behind speech, and a worker kept warm between
    // recordings (live_preview_warm_secs) holds no VRAM while it waits.
    let mut engine_config = config.clone();
    engine_config.engine_isolation = false;
    let engine_model = engine_config.model_name().to_string();
    let (engine_tx, engine_rx) = mpsc::channel();
    let mut start_engine = Some(move || {
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let result = super::create_transcriber(&engine_config);
            eprintln!(
                "[worker] {} loaded in {:.2}s",
                engine_model,
                start.elapsed().as_secs_f32()
            );
            let _ = engine_tx.send(result);
        });
    });

    eprintln!("[worker] Loading live preview model: {}", live_model);
    let load_start = std::time::Instant::now();
    // parakeet-rs's own context sizes. voxtype's [parakeet] streaming_*
    // defaults (1.5 s left context) fail the unified model's check that
    // contexts map to mel-frame counts divisible by 8.
    let streaming_config = UnifiedStreamingConfig::default();
    // None = CPU: the GPU is left to the engine model.
    let unified = super::parakeet::resolve_model_path(live_model)
        .map_err(|e| e.to_string())
        .and_then(|path| ParakeetUnifiedHandle::load(&path, None).map_err(|e| e.to_string()))
        .and_then(|handle| {
            ParakeetUnified::from_shared_with_streaming_config(&handle, streaming_config)
                .map_err(|e| e.to_string())
        });
    let mut unified = match unified {
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

    writeln!(out, "{}", READY_SIGNAL)?;
    out.flush()?;

    let mut samples: Vec<f32> = Vec::new();
    // transcribe_chunk() returns only the text new in that chunk (often
    // empty, sometimes part of a word), so the running transcript is the
    // concatenation.
    let mut last_partial = String::new();
    for block in blocks_rx {
        if let Some(start) = start_engine.take() {
            start();
        }
        samples.extend_from_slice(&block);
        let Some(u) = unified.as_mut() else { continue };
        match u.transcribe_chunk(&block) {
            Ok(delta) if !delta.is_empty() => {
                last_partial.push_str(&delta);
                let shown = last_partial.trim_start();
                writeln!(out, "{}{}", PARTIAL_PREFIX, serde_json::to_string(shown)?)?;
                out.flush()?;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[worker] Live preview failed, stopping it: {}", e);
                unified = None;
            }
        }
    }

    let secs = samples.len() as f32 / 16000.0;
    eprintln!("[worker] Received {:.2}s of audio", secs);
    if samples.is_empty() {
        write_response_to(&mut out, WorkerResponse::error("Empty audio buffer"));
        return Ok(());
    }
    // flush() returns the text still held in the right context.
    if let Some(tail) = unified.as_mut().and_then(|u| u.flush().ok()) {
        last_partial.push_str(&tail);
    }
    let preview_final = last_partial.trim().to_string();

    // A short clip whose engine model is still loading gets the preview
    // model's text: waiting for the load would cost more than it gains.
    let engine = if secs < LIVE_MIN_WAIT_SECS && !preview_final.is_empty() {
        engine_rx.try_recv().ok()
    } else {
        engine_rx.recv().ok()
    };
    let response = match engine {
        Some(Ok(transcriber)) => {
            let start = std::time::Instant::now();
            match transcriber.transcribe(&samples) {
                Ok(text) => {
                    eprintln!(
                        "[worker] Final transcription in {:.2}s",
                        start.elapsed().as_secs_f32()
                    );
                    WorkerResponse::success(text, transcriber.last_detected_language())
                }
                Err(e) if !preview_final.is_empty() => {
                    eprintln!("[worker] Final transcription failed, using preview: {}", e);
                    WorkerResponse::success(preview_final, None)
                }
                Err(e) => WorkerResponse::error(e.to_string()),
            }
        }
        Some(Err(e)) if preview_final.is_empty() => {
            WorkerResponse::error(format!("Failed to load model: {}", e))
        }
        Some(Err(e)) => {
            eprintln!("[worker] Model failed to load, using preview: {}", e);
            WorkerResponse::success(preview_final, None)
        }
        None => {
            eprintln!("[worker] Short clip, model still loading: using preview text");
            WorkerResponse::success(preview_final, None)
        }
    };
    write_response_to(&mut out, response);
    Ok(())
}

#[cfg(not(feature = "parakeet"))]
pub fn run_live_worker(_config: &crate::config::Config, _live_model: &str) -> anyhow::Result<()> {
    anyhow::bail!("live_preview_model needs voxtype built with the parakeet feature")
}

/// Read one `[u32 count][f32 samples]` block. `Ok(None)` at a clean EOF.
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
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
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
