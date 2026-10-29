//! Subprocess-based transcription for GPU isolation
//!
//! This module provides a transcriber that spawns a subprocess for each
//! transcription. When the subprocess exits, all GPU resources are fully
//! released. This solves the problem of GPU memory staying allocated
//! between transcriptions when using ggml-vulkan.
//!
//! Key benefits:
//! - GPU memory fully released after each transcription
//! - No GPU power draw between transcriptions (important for laptops)
//! - Clean separation of concerns
//!
//! Eager spawning:
//! - `prepare()` spawns the worker when recording STARTS
//! - Worker loads model while user is speaking
//! - `transcribe()` sends audio to already-ready worker
//! - Perceived latency is just transcription time, not model load + transcription

use super::streaming::{StreamHandle, StreamingEvent, StreamingTranscriber};
use super::worker::{PARTIAL_PREFIX, READY_SIGNAL, SIGNALS_PREFIX, STATUS_PREFIX, WORDS_PREFIX};
use super::Transcriber;
use crate::config::WhisperConfig;
use crate::error::TranscribeError;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use ureq::serde_json;

/// Response from the transcription worker process
#[derive(Debug, serde::Deserialize)]
struct WorkerResponse {
    ok: bool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error: Option<String>,
    /// Two-letter language code chosen for this transcription, if the worker
    /// tracked it. Used by output methods that benefit from a layout hint.
    /// Older workers that do not emit this field deserialize to `None`.
    #[serde(default)]
    language: Option<String>,
    /// The final engine's text of each spoken correction (live worker).
    #[serde(default)]
    corrections: Vec<crate::osd::correction::FinalClip>,
}

/// A prepared worker process ready to receive audio
struct PreparedWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Subprocess-based transcriber for GPU isolation
///
/// Spawns a fresh `voxtype transcribe-worker` process for each transcription.
/// The worker loads the model, transcribes, returns the result, and exits.
/// This ensures all GPU resources are released after transcription.
///
/// With eager spawning (`prepare()` called when recording starts), the worker
/// loads the model while the user is speaking, hiding load latency.
pub struct SubprocessTranscriber {
    /// Config to pass to the worker
    config: WhisperConfig,
    /// Path to the config file (if any)
    config_path: Option<std::path::PathBuf>,
    /// For a non-Whisper engine: (engine name, model name). The worker reads
    /// the rest of that engine's settings from the config file.
    engine_args: Option<(String, String)>,
    /// Live preview model (`live_preview_model`). When set, the transcriber
    /// streams: see the [`StreamingTranscriber`] impl.
    live_model: Option<String>,
    /// Live worker started ahead of the next recording, and how long it may
    /// wait for one (`live_preview_warm_secs`, 0 = off).
    warm: WarmSlot,
    warm_secs: u64,
    /// `live_preview_warm_release_command`: checked while a warm worker waits.
    warm_release: Option<String>,
    /// Pre-spawned worker (from prepare())
    prepared_worker: Mutex<Option<PreparedWorker>>,
    /// Last language reported by the worker, if any. Mirrors
    /// `WhisperTranscriber::last_language` so the daemon can derive a layout
    /// hint after transcription. See [`Transcriber::last_detected_language`].
    last_language: Arc<Mutex<Option<String>>>,
}

impl SubprocessTranscriber {
    /// Create a new subprocess transcriber
    pub fn new(
        config: &WhisperConfig,
        config_path: Option<std::path::PathBuf>,
    ) -> Result<Self, TranscribeError> {
        Ok(Self {
            config: config.clone(),
            config_path,
            engine_args: None,
            live_model: None,
            warm: WarmSlot::default(),
            warm_secs: 0,
            warm_release: None,
            prepared_worker: Mutex::new(None),
            last_language: Arc::new(Mutex::new(None)),
        })
    }

    /// Create a subprocess transcriber for the configured non-Whisper engine
    /// (`engine_isolation = true`).
    pub fn for_engine(config: &crate::config::Config) -> Self {
        Self {
            config: config.whisper.clone(),
            config_path: crate::config::Config::resolve_existing_path(),
            engine_args: Some((
                config.engine.name().to_string(),
                config.model_name().to_string(),
            )),
            live_model: config
                .live_preview_active()
                .then(|| config.live_preview_model.clone())
                .flatten(),
            warm: WarmSlot::default(),
            warm_secs: config.live_preview_warm_secs,
            warm_release: config.live_preview_warm_release_command.clone(),
            prepared_worker: Mutex::new(None),
            last_language: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the path to the voxtype executable
    fn get_executable_path() -> Result<std::path::PathBuf, TranscribeError> {
        std::env::current_exe().map_err(|e| {
            TranscribeError::InitFailed(format!("Cannot find voxtype executable: {}", e))
        })
    }

    /// Build the command to spawn a worker
    fn build_worker_command(&self) -> Result<Command, TranscribeError> {
        let exe_path = Self::get_executable_path()?;

        let mut cmd = Command::new(&exe_path);

        // Pass config path BEFORE the subcommand — --config is a parent-level
        // arg in clap, so it must appear before "transcribe-worker"
        if let Some(ref config_path) = self.config_path {
            cmd.arg("--config").arg(config_path);
        }

        if let Some((ref engine, ref model)) = self.engine_args {
            // --engine and --model are parent-level args too. ONNX Runtime
            // logs to stderr, so let it through to the daemon's log rather
            // than into a pipe nobody drains.
            cmd.arg("--engine").arg(engine).arg("--model").arg(model);
            cmd.arg("transcribe-worker")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            if let Some(ref live) = self.live_model {
                cmd.arg("--live-model").arg(live);
            }
            return Ok(cmd);
        }

        cmd.arg("transcribe-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Pass essential config via command-line arguments
        cmd.arg("--model").arg(&self.config.model);
        // Serialize language config as comma-separated string for CLI
        // Single: "en", Auto: "auto", Multiple: "en,fr,de"
        let language_str = self.config.language.as_vec().join(",");
        cmd.arg("--language").arg(&language_str);
        if self.config.translate {
            cmd.arg("--translate");
        }
        if let Some(threads) = self.config.threads {
            cmd.arg("--threads").arg(threads.to_string());
        }

        Ok(cmd)
    }

    /// Spawn a worker process and wait for it to be ready
    fn spawn_and_wait_ready(&self) -> Result<PreparedWorker, TranscribeError> {
        let mut cmd = self.build_worker_command()?;

        let mut child = cmd.spawn().map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to spawn transcribe-worker: {}", e))
        })?;

        // Get handles
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TranscribeError::InitFailed("Worker stdin not available".to_string()))?;

        let stdout = child.stdout.take().ok_or_else(|| {
            TranscribeError::InitFailed("Worker stdout not available".to_string())
        })?;

        let mut stdout = BufReader::new(stdout);

        // Wait for READY signal (model loaded)
        let mut ready_line = String::new();
        stdout.read_line(&mut ready_line).map_err(|e| {
            TranscribeError::InitFailed(format!("Failed to read READY signal: {}", e))
        })?;

        if ready_line.trim() != READY_SIGNAL {
            // Worker failed during model load - try to get error from JSON
            if let Ok(response) = serde_json::from_str::<WorkerResponse>(&ready_line) {
                if let Some(error) = response.error {
                    return Err(TranscribeError::InitFailed(error));
                }
            }
            return Err(TranscribeError::InitFailed(format!(
                "Worker failed to load model (got: {:?})",
                ready_line.trim()
            )));
        }

        tracing::debug!("Worker ready (model loaded)");

        Ok(PreparedWorker {
            child,
            stdin,
            stdout,
        })
    }

    /// Write audio samples to the worker's stdin
    fn write_audio_to_worker(
        stdin: &mut ChildStdin,
        samples: &[f32],
    ) -> Result<(), TranscribeError> {
        // Write sample count (u32 little-endian)
        let count = samples.len() as u32;
        stdin.write_all(&count.to_le_bytes()).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to write sample count: {}", e))
        })?;

        // Write samples (f32 little-endian)
        let samples_bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr() as *const u8,
                std::mem::size_of_val(samples),
            )
        };
        stdin.write_all(samples_bytes).map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to write audio samples: {}", e))
        })?;

        stdin.flush().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to flush stdin: {}", e))
        })?;

        Ok(())
    }

    /// Read the JSON response from the worker's stdout
    fn read_worker_response(
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<WorkerResponse, TranscribeError> {
        // A live worker sends preview lines before the response; skip them.
        let mut line = String::new();
        loop {
            line.clear();
            stdout.read_line(&mut line).map_err(|e| {
                TranscribeError::InferenceFailed(format!("Failed to read worker output: {}", e))
            })?;
            if ![PARTIAL_PREFIX, STATUS_PREFIX, WORDS_PREFIX, SIGNALS_PREFIX]
                .iter()
                .any(|p| line.starts_with(p))
            {
                break;
            }
        }

        serde_json::from_str(&line).map_err(|e| {
            TranscribeError::InferenceFailed(format!(
                "Failed to parse worker response: {} (output: {:?})",
                e, line
            ))
        })
    }
}

impl Transcriber for SubprocessTranscriber {
    fn prepare(&self) {
        // A live transcriber spawns its worker in start_stream().
        if self.live_model.is_some() {
            return;
        }
        tracing::debug!("Preparing subprocess transcriber (spawning worker)...");
        let start = std::time::Instant::now();

        match self.spawn_and_wait_ready() {
            Ok(worker) => {
                let mut guard = self.prepared_worker.lock().unwrap();
                *guard = Some(worker);
                tracing::info!(
                    "Worker prepared in {:.2}s (model loaded while recording)",
                    start.elapsed().as_secs_f32()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to prepare worker: {} (will retry on transcribe)", e);
            }
        }
    }

    fn transcribe(&self, samples: &[f32]) -> Result<String, TranscribeError> {
        if samples.is_empty() {
            return Err(TranscribeError::AudioFormat(
                "Empty audio buffer".to_string(),
            ));
        }

        let duration_secs = samples.len() as f32 / 16000.0;

        // Try to use prepared worker, or spawn a new one
        let mut prepared = self.prepared_worker.lock().unwrap();
        let mut worker = match prepared.take() {
            Some(w) => {
                tracing::debug!(
                    "Using pre-spawned worker for {:.2}s of audio",
                    duration_secs
                );
                w
            }
            None => {
                tracing::debug!(
                    "No prepared worker, spawning new one for {:.2}s of audio",
                    duration_secs
                );
                self.spawn_and_wait_ready()?
            }
        };
        drop(prepared); // Release lock

        let start = std::time::Instant::now();

        // Write audio to worker
        Self::write_audio_to_worker(&mut worker.stdin, samples)?;
        drop(worker.stdin); // Close stdin to signal EOF

        // Read response
        let response = Self::read_worker_response(&mut worker.stdout)?;

        // Wait for process to exit
        let status = worker.child.wait().map_err(|e| {
            TranscribeError::InferenceFailed(format!("Failed to wait for worker: {}", e))
        })?;

        if !status.success() {
            // Try to get stderr for error details
            if let Some(mut stderr) = worker.child.stderr.take() {
                let mut err_output = String::new();
                let _ = stderr.read_to_string(&mut err_output);
                if !err_output.is_empty() {
                    tracing::warn!("Worker stderr: {}", err_output.trim());
                }
            }
        }

        tracing::debug!(
            "Subprocess transcription completed in {:.2}s",
            start.elapsed().as_secs_f32()
        );

        // Record reported language for layout-aware output methods. Missing
        // (older worker) leaves the previous value untouched-by-success
        // semantics: clear it on every successful call so stale language
        // hints don't leak across transcriptions when the user switches
        // engines or recordings.
        if let Ok(mut guard) = self.last_language.lock() {
            *guard = response.language.clone();
        }

        // Handle response
        if response.ok {
            response.text.ok_or_else(|| {
                TranscribeError::InferenceFailed("Worker returned ok but no text".to_string())
            })
        } else {
            Err(TranscribeError::InferenceFailed(
                response
                    .error
                    .unwrap_or_else(|| "Unknown worker error".to_string()),
            ))
        }
    }

    fn last_detected_language(&self) -> Option<String> {
        self.last_language.lock().ok().and_then(|g| g.clone())
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.live_model
            .as_ref()
            .map(|_| self as &dyn StreamingTranscriber)
    }
}

/// How often a waiting warm worker runs its release check.
const WARM_RELEASE_CHECK: std::time::Duration = std::time::Duration::from_secs(10);

/// Run the release check (`sh -c`). Success means: stop the warm worker now.
fn release_requested(command: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A live worker started ahead of the next recording, or kept from the last
/// one. It has the preview model loaded, writes READY (read later by
/// `start_stream`'s reader) and waits for audio. The engine model loads only when audio arrives, so it holds no VRAM.
struct WarmWorker {
    child: Child,
    stdin: ChildStdin,
    /// Buffered: a reused worker's READY can already sit in the buffer.
    stdout: BufReader<ChildStdout>,
    generation: u64,
    /// When the worker started loading its live model.
    spawned_at: std::time::Instant,
}

/// Holds at most one warm worker. A timer kills it after the keep time unless
/// a recording took it or a newer one replaced it.
#[derive(Clone, Default)]
struct WarmSlot {
    inner: Arc<Mutex<Option<WarmWorker>>>,
    generation: Arc<AtomicU64>,
}

impl WarmSlot {
    fn take_alive(&self) -> Option<WarmWorker> {
        let mut worker = self.inner.lock().ok()?.take()?;
        match worker.child.try_wait() {
            Ok(None) => Some(worker),
            _ => None,
        }
    }

    fn refill(&self, mut cmd: Command, keep: std::time::Duration, release: Option<String>) {
        let Ok(mut child) = cmd.spawn() else {
            tracing::warn!("Could not start a warm live worker");
            return;
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return;
        };
        let stdout = BufReader::new(stdout);
        self.park(
            child,
            stdin,
            stdout,
            std::time::Instant::now(),
            keep,
            release,
        );
    }

    /// Keep a worker for the next recording, for `keep` at most.
    fn park(
        &self,
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<ChildStdout>,
        spawned_at: std::time::Instant,
        keep: std::time::Duration,
        release: Option<String>,
    ) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let worker = WarmWorker {
            child,
            stdin,
            stdout,
            generation,
            spawned_at,
        };
        if let Ok(mut slot) = self.inner.lock() {
            if let Some(mut old) = slot.replace(worker) {
                let _ = old.child.kill();
                let _ = old.child.wait();
            }
        }
        let inner = Arc::clone(&self.inner);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + keep;
            let reason = loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break format!("idle for {}s", keep.as_secs());
                }
                std::thread::sleep(left.min(WARM_RELEASE_CHECK));
                let still_ours = inner
                    .lock()
                    .map(|slot| slot.as_ref().is_some_and(|w| w.generation == generation))
                    .unwrap_or(false);
                if !still_ours {
                    return;
                }
                if release.as_deref().is_some_and(release_requested) {
                    break "release command said so".to_string();
                }
            };
            let Ok(mut slot) = inner.lock() else { return };
            if slot.as_ref().is_some_and(|w| w.generation == generation) {
                if let Some(mut w) = slot.take() {
                    tracing::info!("Stopping the warm live worker: {}", reason);
                    let _ = w.child.kill();
                    let _ = w.child.wait();
                }
            }
        });
    }
}

/// Live preview: audio goes to the worker in blocks while recording, its
/// running transcript comes back as `Partial` events, and the engine model's
/// transcript of the whole clip arrives as one `Final` after the samples
/// sender is dropped.
impl StreamingTranscriber for SubprocessTranscriber {
    fn start_stream(
        &self,
        mut samples_rx: mpsc::Receiver<Vec<f32>>,
    ) -> Result<StreamHandle, TranscribeError> {
        let (mut child, mut stdin, stdout, spawned_at) = match self.warm.take_alive() {
            Some(w) => {
                tracing::info!("Using the warm live worker");
                (w.child, w.stdin, w.stdout, w.spawned_at)
            }
            None => {
                let mut cmd = self.build_worker_command()?;
                let spawned_at = std::time::Instant::now();
                let mut child = cmd.spawn().map_err(|e| {
                    TranscribeError::InitFailed(format!("Failed to spawn transcribe-worker: {}", e))
                })?;
                let stdin = child.stdin.take().ok_or_else(|| {
                    TranscribeError::InitFailed("Worker stdin not available".to_string())
                })?;
                let stdout = child.stdout.take().ok_or_else(|| {
                    TranscribeError::InitFailed("Worker stdout not available".to_string())
                })?;
                (child, stdin, BufReader::new(stdout), spawned_at)
            }
        };
        let loading = LoadingStatus::start(spawned_at);
        let pid = nix::unistd::Pid::from_raw(child.id() as i32);
        // The next recording's worker: this one if it is still alive when
        // done, else a fresh one.
        let refill = match self.warm_secs {
            0 => None,
            secs => Some((
                self.build_worker_command()?,
                self.warm.clone(),
                std::time::Duration::from_secs(secs),
                self.warm_release.clone(),
            )),
        };

        let (events_tx, events_rx) = mpsc::channel::<StreamingEvent>(64);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        // Audio pump. The worker reads stdin on its own thread from the
        // start, so these writes do not wait for the models to load.
        let (stdin_back_tx, stdin_back_rx) = std::sync::mpsc::channel::<ChildStdin>();
        std::thread::spawn(move || {
            while let Some(chunk) = samples_rx.blocking_recv() {
                if chunk.is_empty() {
                    continue;
                }
                if Self::write_audio_to_worker(&mut stdin, &chunk).is_err() {
                    return;
                }
            }
            // An empty block ends the recording; the worker then stays up
            // for the next one, so stdin goes back for reuse.
            if Self::write_audio_to_worker(&mut stdin, &[]).is_ok() {
                let _ = stdin_back_tx.send(stdin);
            }
        });

        // Cancel kills the worker; the reader below then sees EOF.
        tokio::spawn(async move {
            if cancel_rx.await.is_ok() {
                let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
            }
        });

        let last_language = Arc::clone(&self.last_language);
        let task = tokio::task::spawn_blocking(move || -> Result<(), TranscribeError> {
            let send = |event| {
                let _ = events_tx.blocking_send(event);
            };
            let mut stdout = stdout;
            let mut line = String::new();
            let mut ready = false;
            let mut answered = false;
            loop {
                line.clear();
                match stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let trimmed = line.trim_end();
                if !ready {
                    if trimmed == READY_SIGNAL {
                        ready = true;
                        loading.ready();
                        continue;
                    }
                    // Load failure: the worker replies with an error instead.
                } else if let Some(json) = trimmed.strip_prefix(PARTIAL_PREFIX) {
                    if let Ok(text) = serde_json::from_str::<String>(json) {
                        send(StreamingEvent::Partial {
                            text,
                            segment_id: 0,
                        });
                    }
                    continue;
                } else if let Some(json) = trimmed.strip_prefix(STATUS_PREFIX) {
                    write_live_status(json);
                    continue;
                } else if let Some(json) = trimmed.strip_prefix(WORDS_PREFIX) {
                    if let Ok(lines) = serde_json::from_str(json) {
                        crate::osd::live::word_lines(lines);
                    }
                    continue;
                } else if let Some(json) = trimmed.strip_prefix(SIGNALS_PREFIX) {
                    if let Ok(step) = serde_json::from_str(json) {
                        crate::osd::live::signals(step);
                    }
                    continue;
                }
                match serde_json::from_str::<WorkerResponse>(trimmed) {
                    Ok(r) if r.ok => {
                        if let Ok(mut guard) = last_language.lock() {
                            *guard = r.language.clone();
                        }
                        answered = true;
                        // Before the text, which the daemon corrects with them.
                        crate::osd::correction::final_clips(r.corrections);
                        let text = r.text.unwrap_or_default();
                        if !text.trim().is_empty() {
                            send(StreamingEvent::Final {
                                text,
                                segment_id: 0,
                            });
                        }
                    }
                    Ok(r) => {
                        answered = true;
                        send(StreamingEvent::Error(TranscribeError::InferenceFailed(
                            r.error
                                .unwrap_or_else(|| "Unknown worker error".to_string()),
                        )))
                    }
                    Err(e) => send(StreamingEvent::Error(TranscribeError::InferenceFailed(
                        format!(
                            "Failed to parse worker response: {} (output: {:?})",
                            e, trimmed
                        ),
                    ))),
                }
                break;
            }
            loading.ready();
            // The pump hands stdin back right after the end marker, which
            // the worker needed before it could answer.
            let stdin = answered
                .then(|| {
                    stdin_back_rx
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .ok()
                })
                .flatten();
            match (stdin, refill) {
                (Some(stdin), Some((_, slot, keep, release)))
                    if matches!(child.try_wait(), Ok(None)) =>
                {
                    slot.park(child, stdin, stdout, spawned_at, keep, release);
                }
                (_, refill) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some((cmd, slot, keep, release)) = refill {
                        slot.refill(cmd, keep, release);
                    }
                }
            }
            send(StreamingEvent::Ended);
            Ok(())
        });

        let task = tokio::spawn(async move {
            task.await.unwrap_or_else(|e| {
                Err(TranscribeError::InferenceFailed(format!(
                    "Live worker task panicked: {}",
                    e
                )))
            })
        });

        Ok(StreamHandle {
            events: events_rx,
            cancel: cancel_tx,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_response_parsing() {
        let success: WorkerResponse =
            serde_json::from_str(r#"{"ok": true, "text": "Hello world"}"#).unwrap();
        assert!(success.ok);
        assert_eq!(success.text, Some("Hello world".to_string()));
        // Backward compat: older workers don't emit "language".
        assert_eq!(success.language, None);

        let error: WorkerResponse =
            serde_json::from_str(r#"{"ok": false, "error": "Model not found"}"#).unwrap();
        assert!(!error.ok);
        assert_eq!(error.error, Some("Model not found".to_string()));
    }

    #[test]
    fn test_worker_response_parsing_with_language() {
        let success: WorkerResponse =
            serde_json::from_str(r#"{"ok": true, "text": "Privet", "language": "ru"}"#).unwrap();
        assert!(success.ok);
        assert_eq!(success.language, Some("ru".to_string()));
    }
}

/// Write a worker status line (`{"text", "fraction", "audio_left_s",
/// "eta_s"}`) to the file the OSD shows: the fraction on the first line
/// (empty if unknown), the text on the second. `preview.json` gets all four.
fn write_live_status(json: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let (fraction, text) = (
        v["fraction"].as_f64(),
        v["text"].as_str().unwrap_or_default(),
    );
    write_status_file(fraction, text);
    crate::osd::live::finishing_status(
        crate::osd::live::Phase::Finishing,
        fraction,
        text,
        v["audio_left_s"].as_f64().map(|s| s as f32),
        v["eta_s"].as_f64().map(|s| s as f32),
    );
}

/// How long to wait for READY before telling the user the live model is
/// still loading. A warm worker answers well within this.
const LOADING_STATUS_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// Shows "Loading: ..." in the OSD while the live model loads, with a
/// progress bar from the last measured load time.
struct LoadingStatus {
    state: Arc<Mutex<LoadingState>>,
    spawned_at: std::time::Instant,
}

#[derive(Default)]
struct LoadingState {
    ready: bool,
    shown: bool,
}

impl LoadingStatus {
    fn start(spawned_at: std::time::Instant) -> Self {
        let state = Arc::new(Mutex::new(LoadingState::default()));
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            std::thread::sleep(LOADING_STATUS_DELAY);
            let expected = std::fs::read_to_string(load_secs_path())
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|s| *s > 0.0);
            loop {
                {
                    let Ok(mut st) = shared.lock() else { return };
                    if st.ready {
                        return;
                    }
                    let secs = spawned_at.elapsed().as_secs_f64();
                    // Never show a full bar: the last load time is a guess.
                    let fraction = expected.map(|e| (secs / e).min(0.95));
                    let text = format!("Loading: live text model, {:.0} s", secs);
                    write_status_file(fraction, &text);
                    crate::osd::live::status(crate::osd::live::Phase::Loading, fraction, &text);
                    st.shown = true;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        });
        Self { state, spawned_at }
    }

    /// The live model is loaded, or the worker ended. Clears the status if
    /// this showed it, and remembers the load time for the next bar.
    fn ready(&self) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if st.ready {
            return;
        }
        st.ready = true;
        if st.shown {
            let _ = std::fs::remove_file(crate::config::Config::live_status_path());
            crate::osd::live::clear_status();
            let secs = self.spawned_at.elapsed().as_secs_f64();
            let _ = std::fs::write(load_secs_path(), format!("{:.2}", secs));
        }
    }
}

/// Last live model load time, read by the next loading bar.
fn load_secs_path() -> std::path::PathBuf {
    crate::config::Config::live_status_path().with_file_name("live-load-secs")
}

fn write_status_file(fraction: Option<f64>, text: &str) {
    let fraction = fraction.map(|f| f.to_string()).unwrap_or_default();
    let path = crate::config::Config::live_status_path();
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, format!("{}\n{}", fraction, text)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
