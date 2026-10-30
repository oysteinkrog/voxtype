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
    /// `live_preview_warm_idle_secs` or `live_preview_warm_presence_file`:
    /// keep a worker while the user is present instead. Started by
    /// [`Transcriber::keep_warm`].
    keep_policy: Option<KeepPolicy>,
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
            keep_policy: None,
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
            keep_policy: KeepPolicy::from_config(config),
            warm_release: config.live_preview_warm_release_command.clone(),
            prepared_worker: Mutex::new(None),
            last_language: Arc::new(Mutex::new(None)),
        }
    }

    /// Build the command to spawn a worker
    fn build_worker_command(&self) -> Result<Command, TranscribeError> {
        worker_command(
            &self.config,
            self.config_path.as_deref(),
            self.engine_args.as_ref(),
            self.live_model.as_deref(),
        )
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

/// A command that runs this same voxtype binary again, for a worker.
///
/// On Linux it executes `/proc/self/exe`, which names the running binary
/// even after a package upgrade replaced or deleted its file; a path from
/// `current_exe()` then points at nothing (or at a newer build), and every
/// worker spawn failed with "No such file or directory". `argv[0]` stays the
/// binary's path, as before, since ONNX Runtime looks for its provider
/// libraries next to it.
pub fn self_command() -> std::io::Result<Command> {
    let exe = std::env::current_exe();
    #[cfg(target_os = "linux")]
    {
        let proc_exe = std::path::Path::new("/proc/self/exe");
        if proc_exe.exists() {
            use std::os::unix::process::CommandExt;
            let mut cmd = Command::new(proc_exe);
            if let Ok(ref path) = exe {
                // After a replace the link reads "<path> (deleted)".
                let path = path.to_string_lossy();
                cmd.arg0(path.strip_suffix(" (deleted)").unwrap_or(&path));
            }
            return Ok(cmd);
        }
    }
    Ok(Command::new(exe?))
}

/// The command that starts a `transcribe-worker`.
fn worker_command(
    config: &WhisperConfig,
    config_path: Option<&std::path::Path>,
    engine_args: Option<&(String, String)>,
    live_model: Option<&str>,
) -> Result<Command, TranscribeError> {
    let mut cmd = self_command().map_err(|e| {
        TranscribeError::InitFailed(format!("Cannot find voxtype executable: {}", e))
    })?;

    // Pass config path BEFORE the subcommand — --config is a parent-level
    // arg in clap, so it must appear before "transcribe-worker"
    if let Some(config_path) = config_path {
        cmd.arg("--config").arg(config_path);
    }

    if let Some((engine, model)) = engine_args {
        // --engine and --model are parent-level args too. ONNX Runtime
        // logs to stderr, so let it through to the daemon's log rather
        // than into a pipe nobody drains.
        cmd.arg("--engine").arg(engine).arg("--model").arg(model);
        cmd.arg("transcribe-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(live) = live_model {
            cmd.arg("--live-model").arg(live);
        }
        return Ok(cmd);
    }

    cmd.arg("transcribe-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Pass essential config via command-line arguments
    cmd.arg("--model").arg(&config.model);
    // Serialize language config as comma-separated string for CLI
    // Single: "en", Auto: "auto", Multiple: "en,fr,de"
    let language_str = config.language.as_vec().join(",");
    cmd.arg("--language").arg(&language_str);
    if config.translate {
        cmd.arg("--translate");
    }
    if let Some(threads) = config.threads {
        cmd.arg("--threads").arg(threads.to_string());
    }

    Ok(cmd)
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

    fn keep_warm(&self) {
        if self.live_model.is_none() {
            return;
        }
        if let Some(ref policy) = self.keep_policy {
            let config = self.config.clone();
            let config_path = self.config_path.clone();
            let engine_args = self.engine_args.clone();
            let live_model = self.live_model.clone();
            self.warm.start_keeper(
                move || {
                    worker_command(
                        &config,
                        config_path.as_deref(),
                        engine_args.as_ref(),
                        live_model.as_deref(),
                    )
                },
                policy.clone(),
                self.warm_release.clone(),
            );
        } else if self.warm_secs > 0 && self.warm.is_empty() {
            match self.build_worker_command() {
                Ok(cmd) => {
                    tracing::info!(
                        "Loading the warm live worker (daemon start), kept for {}s",
                        self.warm_secs
                    );
                    self.warm.refill(
                        cmd,
                        std::time::Duration::from_secs(self.warm_secs),
                        self.warm_release.clone(),
                    );
                }
                Err(e) => tracing::warn!("Could not start a warm live worker: {}", e),
            }
        }
    }

    fn as_streaming(&self) -> Option<&dyn StreamingTranscriber> {
        self.live_model
            .as_ref()
            .map(|_| self as &dyn StreamingTranscriber)
    }
}

/// How often a waiting warm worker runs its release check.
const WARM_RELEASE_CHECK: std::time::Duration = std::time::Duration::from_secs(10);

/// How often the presence keeper looks at a waiting worker, to load a new one
/// if it died.
const KEEPER_CHECK: std::time::Duration = std::time::Duration::from_secs(60);

/// While the user is away, the keeper still wakes this often, so it ends
/// soon after its transcriber is dropped.
const KEEPER_AWAY_CHECK: std::time::Duration = std::time::Duration::from_secs(300);

/// The keeper stops reloading after this many warm workers in a row exited
/// on their own, until the next recording.
const KEEPER_MAX_EXITS: u32 = 3;

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

impl WarmWorker {
    fn spawn(mut cmd: Command) -> Option<(Child, ChildStdin, BufReader<ChildStdout>)> {
        let Ok(mut child) = cmd.spawn() else {
            tracing::warn!("Could not start a warm live worker");
            return None;
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        };
        Some((child, stdin, BufReader::new(stdout)))
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Whether the worker has said READY, without reading it.
    fn loaded(&self) -> bool {
        if !self.stdout.buffer().is_empty() {
            return true;
        }
        let mut pfd = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(self.stdout.get_ref()),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut pfd, 1, 0) > 0 }
    }
}

/// Why the slot is empty: named in the log line of the next load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptyBecause {
    /// Nothing loaded one yet.
    Start,
    /// Released after the user was away.
    Away,
    /// Released because the release command asked.
    ReleaseCommand,
    /// A recording used the worker and could not hand it back.
    Consumed,
    /// The worker exited on its own.
    Exited,
}

#[derive(Default)]
struct SlotState {
    worker: Option<WarmWorker>,
    /// A recording holds the worker (presence keeper only).
    busy: bool,
    why_empty: Option<EmptyBecause>,
    /// Recordings that ended, so the keeper can tell a working worker from
    /// one that keeps dying.
    recordings: u64,
}

#[derive(Default)]
struct SlotShared {
    state: Mutex<SlotState>,
    generation: AtomicU64,
    /// Wakes the presence keeper; set once one runs for this slot.
    keeper: std::sync::OnceLock<std::sync::mpsc::Sender<()>>,
}

impl Drop for SlotShared {
    fn drop(&mut self) {
        if let Some(w) = self.state.get_mut().ok().and_then(|st| st.worker.take()) {
            w.stop();
        }
    }
}

/// Holds at most one warm worker. Either a timer kills it after the keep
/// time (`live_preview_warm_secs`), or the presence keeper keeps it while the
/// user is at the computer (`live_preview_warm_idle_secs`).
#[derive(Clone, Default)]
struct WarmSlot(Arc<SlotShared>);

/// What the presence keeper does next.
#[derive(Debug, PartialEq, Eq)]
enum WarmAction {
    Load,
    Release,
    Keep,
}

/// The presence keeper's rules.
#[derive(Clone, Debug)]
struct KeepPolicy {
    /// Without a trusted presence file: release after this long with no
    /// input (`live_preview_warm_idle_secs`).
    idle_limit: std::time::Duration,
    /// `live_preview_warm_presence_file`.
    presence_file: Option<std::path::PathBuf>,
    /// `live_preview_warm_presence_keep`: states that keep a worker.
    keep_states: Vec<String>,
    /// `live_preview_warm_presence_grace_secs`: how long another state
    /// (and no local input) lasts before the worker goes.
    grace: std::time::Duration,
    /// How often the keeper reads the presence file.
    file_check: std::time::Duration,
}

/// The input timer's limit when only the presence file is set.
const DEFAULT_IDLE_FALLBACK: std::time::Duration = std::time::Duration::from_secs(1800);

/// How often the keeper reads the presence file.
const PRESENCE_FILE_CHECK: std::time::Duration = std::time::Duration::from_secs(2);

impl KeepPolicy {
    /// None when neither `live_preview_warm_idle_secs` nor
    /// `live_preview_warm_presence_file` is set.
    fn from_config(config: &crate::config::Config) -> Option<Self> {
        let presence_file = config
            .live_preview_warm_presence_file
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(super::revisions::expand_home);
        let idle = config.live_preview_warm_idle_secs;
        if idle == 0 && presence_file.is_none() {
            return None;
        }
        Some(Self {
            idle_limit: match idle {
                0 => DEFAULT_IDLE_FALLBACK,
                secs => std::time::Duration::from_secs(secs),
            },
            presence_file,
            keep_states: config.live_preview_warm_presence_keep.clone(),
            grace: std::time::Duration::from_secs(config.live_preview_warm_presence_grace_secs),
            file_check: PRESENCE_FILE_CHECK,
        })
    }
}

/// What the presence rules say now: keep a worker or not, and why.
#[derive(Debug, PartialEq, Eq)]
struct Want {
    keep: bool,
    why: String,
}

/// With a trusted presence file: keep a worker in a keep state, and in any
/// other state until it (and the quiet on this machine's own keyboard and
/// mouse) lasted the grace period. Remote-desktop input does not count
/// then. Without one: keep while there was any input within the idle limit.
fn want_warm(
    policy: &KeepPolicy,
    file: Option<&crate::presence::FileReading>,
    idle_for: std::time::Duration,
    local_idle_for: std::time::Duration,
) -> Want {
    match file {
        Some(f) if policy.keep_states.contains(&f.state) => Want {
            keep: true,
            why: format!("presence {}: {}", f.state, f.reason),
        },
        Some(f) => {
            let away_for = f
                .since_age
                .map_or(local_idle_for, |age| age.min(local_idle_for));
            Want {
                keep: away_for < policy.grace,
                why: format!(
                    "presence {} for {}s: {}",
                    f.state,
                    away_for.as_secs(),
                    f.reason
                ),
            }
        }
        None => Want {
            keep: idle_for < policy.idle_limit,
            why: format!("no keyboard or mouse input for {}s", idle_for.as_secs()),
        },
    }
}

/// Load a worker when one is wanted and none waits, release it when it is
/// not wanted or the release command asks for the memory.
fn warm_action(wanted: bool, has_worker: bool, busy: bool, release_says: bool) -> WarmAction {
    if busy {
        return WarmAction::Keep;
    }
    match (has_worker, wanted && !release_says) {
        (true, false) => WarmAction::Release,
        (false, true) => WarmAction::Load,
        _ => WarmAction::Keep,
    }
}

/// How long the keeper may sleep. With a presence file, until its next
/// read. Otherwise until the user counts as away, and at most
/// [`KEEPER_CHECK`] (or [`WARM_RELEASE_CHECK`] with a release command)
/// while they are here; while away it waits for their return.
fn keeper_wait(
    policy: &KeepPolicy,
    idle_for: std::time::Duration,
    release_command: bool,
) -> std::time::Duration {
    if policy.presence_file.is_some() {
        return policy.file_check;
    }
    if idle_for >= policy.idle_limit {
        return KEEPER_AWAY_CHECK;
    }
    let check = if release_command {
        WARM_RELEASE_CHECK
    } else {
        KEEPER_CHECK
    };
    (policy.idle_limit - idle_for)
        .min(check)
        .max(std::time::Duration::from_millis(100))
}

impl WarmSlot {
    fn presence_mode(&self) -> bool {
        self.0.keeper.get().is_some()
    }

    fn is_empty(&self) -> bool {
        self.0
            .state
            .lock()
            .map(|st| st.worker.is_none())
            .unwrap_or(true)
    }

    fn wake_keeper(&self) {
        if let Some(tx) = self.0.keeper.get() {
            let _ = tx.send(());
        }
    }

    /// The waiting worker, if it is still alive. With the presence keeper,
    /// the slot counts as busy until [`WarmSlot::hand_back`], so the keeper
    /// does not load a second worker meanwhile.
    fn take_alive(&self) -> Option<WarmWorker> {
        let mut st = self.0.state.lock().ok()?;
        if self.presence_mode() {
            st.busy = true;
        }
        let worker = st.worker.take()?;
        st.why_empty = Some(EmptyBecause::Consumed);
        let mut worker = worker;
        match worker.child.try_wait() {
            Ok(None) => Some(worker),
            _ => {
                worker.stop();
                None
            }
        }
    }

    /// Presence keeper: the recording is over. Keep its worker if it is
    /// still usable; otherwise the keeper loads a new one.
    fn hand_back(
        &self,
        worker: Option<(
            Child,
            ChildStdin,
            BufReader<ChildStdout>,
            std::time::Instant,
        )>,
    ) {
        if let Ok(mut st) = self.0.state.lock() {
            st.busy = false;
            st.recordings += 1;
            if let Some((child, stdin, stdout, spawned_at)) = worker {
                tracing::info!("Keeping the live worker warm for the next recording");
                let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(old) = st.worker.replace(WarmWorker {
                    child,
                    stdin,
                    stdout,
                    generation,
                    spawned_at,
                }) {
                    old.stop();
                }
                st.why_empty = None;
            }
        }
        self.wake_keeper();
    }

    fn refill(&self, cmd: Command, keep: std::time::Duration, release: Option<String>) {
        let Some((child, stdin, stdout)) = WarmWorker::spawn(cmd) else {
            return;
        };
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
        let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let worker = WarmWorker {
            child,
            stdin,
            stdout,
            generation,
            spawned_at,
        };
        if let Ok(mut st) = self.0.state.lock() {
            if let Some(old) = st.worker.replace(worker) {
                old.stop();
            }
        }
        let shared = Arc::downgrade(&self.0);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + keep;
            let reason = loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break format!("idle for {}s", keep.as_secs());
                }
                std::thread::sleep(left.min(WARM_RELEASE_CHECK));
                let still_ours = shared.upgrade().is_some_and(|s| {
                    s.state.lock().is_ok_and(|st| {
                        st.worker
                            .as_ref()
                            .is_some_and(|w| w.generation == generation)
                    })
                });
                if !still_ours {
                    return;
                }
                if release.as_deref().is_some_and(release_requested) {
                    break "release command said so".to_string();
                }
            };
            let Some(shared) = shared.upgrade() else {
                return;
            };
            let Ok(mut st) = shared.state.lock() else {
                return;
            };
            if st
                .worker
                .as_ref()
                .is_some_and(|w| w.generation == generation)
            {
                if let Some(w) = st.worker.take() {
                    tracing::info!("Stopping the warm live worker: {}", reason);
                    w.stop();
                }
            }
        });
    }

    /// Start the presence keeper: from now on a worker stays loaded while
    /// `policy` says the user is here. Does nothing if one runs.
    fn start_keeper(
        &self,
        command: impl Fn() -> Result<Command, TranscribeError> + Send + 'static,
        policy: KeepPolicy,
        release: Option<String>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.0.keeper.set(tx.clone()).is_err() {
            return;
        }
        let presence = crate::presence::global();
        presence.watch(tx);
        let shared = Arc::downgrade(&self.0);
        let spawned = std::thread::Builder::new()
            .name("voxtype-warm".into())
            .spawn(move || keep_warm(shared, rx, presence, command, policy, release));
        if let Err(e) = spawned {
            tracing::warn!("Cannot keep a live worker warm: {}", e);
        }
    }
}

/// The presence keeper's loop. See [`WarmSlot::start_keeper`].
fn keep_warm(
    shared: std::sync::Weak<SlotShared>,
    wake: std::sync::mpsc::Receiver<()>,
    presence: Arc<crate::presence::Presence>,
    command: impl Fn() -> Result<Command, TranscribeError>,
    policy: KeepPolicy,
    release: Option<String>,
) {
    match policy.presence_file {
        Some(ref path) => tracing::info!(
            "Keeping a live worker warm while {} says {:?} (else released after {}s); \
             without a fresh file, released after {}s without input",
            path.display(),
            policy.keep_states,
            policy.grace.as_secs(),
            policy.idle_limit.as_secs()
        ),
        None => tracing::info!(
            "Keeping a live worker warm while you use the computer (released after {}s without input)",
            policy.idle_limit.as_secs()
        ),
    }
    let mut release_says = false;
    let mut release_checked: Option<std::time::Instant> = None;
    // Workers that exited on their own in a row, and the recording count
    // when the last one did.
    let mut exits = 0u32;
    let mut exits_at_recording = 0u64;
    // The presence file's last state (None: missing or stale), for the log.
    let mut seen: Option<Option<String>> = None;
    loop {
        let Some(slot) = shared.upgrade() else {
            return;
        };
        let now = std::time::Instant::now();
        let (last, kind) = presence.last();
        let (last_local, kind_local) = presence.last_local();
        let idle_for = now.saturating_duration_since(last);
        let local_idle_for = now.saturating_duration_since(last_local);
        let file = policy
            .presence_file
            .as_deref()
            .and_then(crate::presence::read_presence_file);
        if policy.presence_file.is_some() {
            let state = file.as_ref().map(|f| f.state.clone());
            if seen.as_ref() != Some(&state) {
                match file {
                    Some(ref f) => tracing::info!("Presence: {} ({})", f.state, f.reason),
                    None => tracing::info!(
                        "Presence file missing or stale; going by keyboard and mouse input"
                    ),
                }
                seen = Some(state);
            }
        }
        let want = want_warm(&policy, file.as_ref(), idle_for, local_idle_for);
        if let Some(ref cmd) = release {
            let due = release_checked.is_none_or(|t| now.duration_since(t) >= WARM_RELEASE_CHECK);
            if want.keep && due {
                release_says = release_requested(cmd);
                release_checked = Some(now);
            }
        }
        {
            let Ok(mut st) = slot.state.lock() else {
                return;
            };
            if let Some(w) = st.worker.as_mut() {
                if let Ok(Some(status)) = w.child.try_wait() {
                    tracing::warn!("The warm live worker exited on its own ({})", status);
                    st.worker = None;
                    st.why_empty = Some(EmptyBecause::Exited);
                    if st.recordings != exits_at_recording {
                        exits = 0;
                    }
                    exits += 1;
                    exits_at_recording = st.recordings;
                    if exits == KEEPER_MAX_EXITS {
                        tracing::warn!(
                            "{} warm live workers in a row exited; not loading another until the next recording",
                            exits
                        );
                    }
                }
            }
            let gave_up = exits >= KEEPER_MAX_EXITS && st.recordings == exits_at_recording;
            match warm_action(want.keep, st.worker.is_some(), st.busy, release_says) {
                WarmAction::Load if gave_up => {}
                WarmAction::Load => {
                    let back = match file {
                        Some(ref f) if policy.keep_states.contains(&f.state) => want.why.clone(),
                        Some(_) => kind_local.describe().to_string(),
                        None => kind.describe().to_string(),
                    };
                    let reason = match st.why_empty.unwrap_or(EmptyBecause::Start) {
                        EmptyBecause::Start => "daemon start".to_string(),
                        EmptyBecause::Away => format!("you are back: {}", back),
                        EmptyBecause::ReleaseCommand => {
                            "the release command no longer asks for the memory".to_string()
                        }
                        EmptyBecause::Consumed => "the last recording used it up".to_string(),
                        EmptyBecause::Exited => "the last one exited".to_string(),
                    };
                    let spawned = command().ok().and_then(WarmWorker::spawn);
                    if let Some((child, stdin, stdout)) = spawned {
                        tracing::info!("Loading the warm live worker ({})", reason);
                        let generation = slot.generation.fetch_add(1, Ordering::SeqCst) + 1;
                        st.worker = Some(WarmWorker {
                            child,
                            stdin,
                            stdout,
                            generation,
                            spawned_at: now,
                        });
                        st.why_empty = None;
                    }
                }
                WarmAction::Release => {
                    if let Some(w) = st.worker.take() {
                        w.stop();
                    }
                    if release_says {
                        tracing::info!("Releasing the warm live worker: the release command asked");
                        st.why_empty = Some(EmptyBecause::ReleaseCommand);
                    } else {
                        tracing::info!("Releasing the warm live worker: {}", want.why);
                        st.why_empty = Some(EmptyBecause::Away);
                    }
                }
                WarmAction::Keep => {}
            }
        }
        drop(slot);
        let wait = keeper_wait(&policy, idle_for, release.is_some());
        if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) = wake.recv_timeout(wait) {
            return;
        }
    }
}

/// What happens to a live worker after its recording.
enum Refill {
    /// Stop it.
    None,
    /// `live_preview_warm_secs`: keep it (or a fresh one) for this long.
    Timer(Box<Command>, WarmSlot, std::time::Duration, Option<String>),
    /// `live_preview_warm_idle_secs`: give it back to the presence keeper.
    Keeper(WarmSlot),
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
        let kept = self.warm.presence_mode();
        let taken = self.warm.take_alive();
        if kept {
            crate::presence::global().mark(crate::presence::Activity::Recording);
        }
        let (mut child, mut stdin, stdout, spawned_at) = match taken {
            Some(w) => {
                if w.loaded() {
                    tracing::info!(
                        "Using the warm live worker (loaded, started {:.1}s ago)",
                        w.spawned_at.elapsed().as_secs_f32()
                    );
                } else {
                    tracing::info!(
                        "Using the warm live worker (still loading, started {:.1}s ago)",
                        w.spawned_at.elapsed().as_secs_f32()
                    );
                }
                (w.child, w.stdin, w.stdout, w.spawned_at)
            }
            None => {
                tracing::info!("No warm live worker; loading the live model now");
                let spawned = (|| {
                    let mut cmd = self.build_worker_command()?;
                    let spawned_at = std::time::Instant::now();
                    let mut child = cmd.spawn().map_err(|e| {
                        TranscribeError::InitFailed(format!(
                            "Failed to spawn transcribe-worker: {}",
                            e
                        ))
                    })?;
                    let stdin = child.stdin.take().ok_or_else(|| {
                        TranscribeError::InitFailed("Worker stdin not available".to_string())
                    })?;
                    let stdout = child.stdout.take().ok_or_else(|| {
                        TranscribeError::InitFailed("Worker stdout not available".to_string())
                    })?;
                    Ok((child, stdin, BufReader::new(stdout), spawned_at))
                })();
                match spawned {
                    Ok(w) => w,
                    Err(e) => {
                        if kept {
                            self.warm.hand_back(None);
                        }
                        return Err(e);
                    }
                }
            }
        };
        let loading = LoadingStatus::start(spawned_at);
        let pid = nix::unistd::Pid::from_raw(child.id() as i32);
        // The next recording's worker: this one if it is still alive when
        // done, else a fresh one.
        let refill = if kept {
            Refill::Keeper(self.warm.clone())
        } else {
            match self.warm_secs {
                0 => Refill::None,
                secs => Refill::Timer(
                    Box::new(self.build_worker_command()?),
                    self.warm.clone(),
                    std::time::Duration::from_secs(secs),
                    self.warm_release.clone(),
                ),
            }
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
            let alive = matches!(child.try_wait(), Ok(None));
            match (stdin.filter(|_| alive), refill) {
                (Some(stdin), Refill::Timer(_, slot, keep, release)) => {
                    slot.park(child, stdin, stdout, spawned_at, keep, release);
                }
                (Some(stdin), Refill::Keeper(slot)) => {
                    slot.hand_back(Some((child, stdin, stdout, spawned_at)));
                }
                (_, refill) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    match refill {
                        Refill::Timer(cmd, slot, keep, release) => slot.refill(*cmd, keep, release),
                        Refill::Keeper(slot) => slot.hand_back(None),
                        Refill::None => {}
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

    use crate::presence::{Activity, Presence};
    use std::time::{Duration, Instant};

    const HOUR: Duration = Duration::from_secs(3600);

    fn policy(file: Option<std::path::PathBuf>) -> KeepPolicy {
        KeepPolicy {
            idle_limit: HOUR,
            presence_file: file,
            keep_states: vec!["here".to_string()],
            grace: Duration::from_secs(600),
            file_check: Duration::from_millis(20),
        }
    }

    fn reading(state: &str, since_age: u64) -> crate::presence::FileReading {
        crate::presence::FileReading {
            state: state.to_string(),
            since_age: Some(Duration::from_secs(since_age)),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn the_keeper_acts_on_what_is_wanted() {
        assert_eq!(warm_action(true, false, false, false), WarmAction::Load);
        assert_eq!(warm_action(true, true, false, false), WarmAction::Keep);
        assert_eq!(warm_action(false, true, false, false), WarmAction::Release);
        assert_eq!(warm_action(false, false, false, false), WarmAction::Keep);
        // A recording holds the worker: never load a second one.
        assert_eq!(warm_action(true, false, true, false), WarmAction::Keep);
        // The release command wins over presence.
        assert_eq!(warm_action(true, true, false, true), WarmAction::Release);
        assert_eq!(warm_action(true, false, false, true), WarmAction::Keep);
    }

    #[test]
    fn without_a_presence_file_input_decides() {
        let p = policy(None);
        let s = Duration::from_secs;
        assert!(want_warm(&p, None, s(10), s(10)).keep);
        let away = want_warm(&p, None, HOUR, HOUR);
        assert!(!away.keep);
        assert_eq!(away.why, "no keyboard or mouse input for 3600s");
    }

    #[test]
    fn a_keep_state_keeps_even_without_input() {
        let p = policy(None);
        let w = want_warm(&p, Some(&reading("here", 9000)), HOUR * 5, HOUR * 5);
        assert!(w.keep);
        assert_eq!(w.why, "presence here: test");
    }

    #[test]
    fn another_state_releases_after_the_grace() {
        let p = policy(None);
        let s = Duration::from_secs;
        assert!(want_warm(&p, Some(&reading("away", 599)), HOUR, HOUR).keep);
        let w = want_warm(&p, Some(&reading("away", 601)), HOUR, HOUR);
        assert!(!w.keep);
        assert_eq!(w.why, "presence away for 601s: test");
        // Remote-desktop input (only `idle_for` is recent) keeps nothing.
        assert!(!want_warm(&p, Some(&reading("remote", 3000)), s(1), HOUR).keep);
        // Local keyboard or mouse input starts the grace again.
        assert!(want_warm(&p, Some(&reading("remote", 3000)), s(1), s(1)).keep);
    }

    #[test]
    fn the_keeper_wakes_when_the_user_turns_away() {
        let s = Duration::from_secs;
        let mut p = policy(None);
        p.idle_limit = s(1800);
        assert_eq!(keeper_wait(&p, s(1790), false), s(10));
        assert_eq!(keeper_wait(&p, s(0), false), KEEPER_CHECK);
        assert_eq!(keeper_wait(&p, s(0), true), WARM_RELEASE_CHECK);
        assert_eq!(keeper_wait(&p, s(1800), false), KEEPER_AWAY_CHECK);
        // With a presence file, it reads the file on its own beat.
        let p = policy(Some("/x".into()));
        assert_eq!(keeper_wait(&p, s(1800), false), p.file_check);
    }

    /// A stand-in worker: waits on stdin like a loaded live worker.
    fn fake_worker() -> Result<Command, TranscribeError> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("echo READY; cat > /dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok(cmd)
    }

    fn worker_pid(slot: &WarmSlot) -> Option<u32> {
        let st = slot.0.state.lock().unwrap();
        st.worker.as_ref().map(|w| w.child.id())
    }

    /// Wait up to 5 s for the keeper to reach `want`.
    fn wait_for(slot: &WarmSlot, want: impl Fn(Option<u32>) -> bool) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pid = worker_pid(slot);
            if want(pid) || Instant::now() > deadline {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn start_test_keeper(slot: &WarmSlot, presence: &Arc<Presence>, policy: KeepPolicy) {
        let (tx, rx) = std::sync::mpsc::channel();
        slot.0.keeper.set(tx.clone()).unwrap();
        presence.watch(tx);
        let shared = Arc::downgrade(&slot.0);
        let presence = Arc::clone(presence);
        std::thread::spawn(move || keep_warm(shared, rx, presence, fake_worker, policy, None));
    }

    /// Write a presence file as presence-detect does: `updated` now,
    /// `since` that many seconds ago (`updated_ago` makes it stale).
    fn write_presence(path: &std::path::Path, state: &str, since_ago: f64, updated_ago: f64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let tmp = path.with_extension("tmp");
        std::fs::write(
            &tmp,
            format!(
                r#"{{"state": "{}", "since": {}, "updated": {}, "reason": "test {}"}}"#,
                state,
                now - since_ago,
                now - updated_ago,
                state
            ),
        )
        .unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    #[test]
    fn the_keeper_follows_the_presence_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("state");
        write_presence(&file, "here", 10.0, 0.0);
        let presence = Arc::new(Presence::new(Instant::now()));
        let slot = WarmSlot::default();
        start_test_keeper(&slot, &presence, policy(Some(file.clone())));

        // here: loaded.
        let first = wait_for(&slot, |p| p.is_some());
        assert!(first.is_some(), "loaded while here");

        // away past the grace, nobody at the local keyboard: released.
        presence.set_last(Instant::now() - HOUR, Activity::Input);
        write_presence(&file, "away", 700.0, 0.0);
        assert_eq!(wait_for(&slot, |p| p.is_none()), None, "released when away");

        // remote, with remote-desktop input: stays released.
        write_presence(&file, "remote", 3000.0, 0.0);
        presence.mark(Activity::RemoteInput);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(worker_pid(&slot), None, "remote input keeps nothing warm");

        // here again: loaded at once.
        write_presence(&file, "here", 0.0, 0.0);
        let back = wait_for(&slot, |p| p.is_some());
        assert!(back.is_some() && back != first, "reloaded on here");

        // A stale file: the input rule decides. No input for over the idle
        // limit, so the worker goes.
        write_presence(&file, "here", 0.0, 120.0);
        presence.set_last(
            Instant::now() - HOUR - Duration::from_secs(1),
            Activity::Input,
        );
        assert_eq!(
            wait_for(&slot, |p| p.is_none()),
            None,
            "stale file: input rule"
        );

        // Still stale: the next local input loads it again.
        presence.mark(Activity::Input);
        assert!(
            wait_for(&slot, |p| p.is_some()).is_some(),
            "stale file: reload on input"
        );
    }

    #[test]
    fn the_keeper_follows_the_user() {
        let presence = Arc::new(Presence::new(Instant::now()));
        let slot = WarmSlot::default();
        start_test_keeper(&slot, &presence, policy(None));

        // Daemon start: a worker loads at once.
        let first = wait_for(&slot, |p| p.is_some()).expect("loaded at start");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !slot
            .0
            .state
            .lock()
            .unwrap()
            .worker
            .as_ref()
            .unwrap()
            .loaded()
        {
            assert!(Instant::now() < deadline, "the stand-in never said READY");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Away for over the limit: released.
        presence.set_last(
            Instant::now() - HOUR - Duration::from_secs(1),
            Activity::Input,
        );
        slot.wake_keeper();
        assert_eq!(wait_for(&slot, |p| p.is_none()), None, "released when away");

        // Back: the first input loads one again.
        presence.mark(Activity::Input);
        let back = wait_for(&slot, |p| p.is_some());
        assert!(back.is_some(), "reloaded on input");
        assert_ne!(back, Some(first));

        // A recording takes it; no second worker loads meanwhile.
        let taken = slot.take_alive().expect("a live worker");
        slot.wake_keeper();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            worker_pid(&slot),
            None,
            "no second worker during a recording"
        );

        // The recording hands it back: kept.
        slot.hand_back(Some((
            taken.child,
            taken.stdin,
            taken.stdout,
            taken.spawned_at,
        )));
        assert_eq!(worker_pid(&slot), back);

        // A recording that used it up: the keeper loads a new one.
        let used = slot.take_alive().unwrap();
        used.stop();
        slot.hand_back(None);
        let after = wait_for(&slot, |p| p.is_some_and(|p| Some(p) != back));
        assert!(after.is_some() && after != back);

        // A worker that dies on its own is replaced.
        let pid = after.unwrap();
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
        let deadline = Instant::now() + Duration::from_secs(5);
        let replaced = loop {
            slot.wake_keeper();
            let now = worker_pid(&slot);
            if now.is_some_and(|p| p != pid) || Instant::now() > deadline {
                break now;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(replaced.is_some_and(|p| p != pid), "replaced after it died");
    }

    #[test]
    fn dropping_the_slot_stops_its_worker() {
        let slot = WarmSlot::default();
        let (child, stdin, stdout) = WarmWorker::spawn(fake_worker().unwrap()).unwrap();
        let pid = child.id();
        slot.hand_back(Some((child, stdin, stdout, Instant::now())));
        drop(slot);
        // Killed and reaped by the drop, so the pid is gone.
        let pid = nix::unistd::Pid::from_raw(pid as i32);
        assert!(nix::sys::signal::kill(pid, None).is_err());
    }

    /// Env var that makes [`self_command_survives_a_deleted_binary`] the
    /// harness inside a linked copy of the test binary.
    const SELF_EXEC_HARNESS: &str = "VOXTYPE_TEST_SELF_EXEC_HARNESS";

    /// A package upgrade replaces the binary under a running daemon. Run a
    /// hard-linked copy of this test binary, delete the copy's file from
    /// inside it, then spawn "itself" again: the path from current_exe() is
    /// gone, /proc/self/exe still works.
    #[test]
    fn self_command_survives_a_deleted_binary() {
        if std::env::var_os(SELF_EXEC_HARNESS).is_some() {
            let exe = std::env::current_exe().unwrap();
            std::fs::remove_file(&exe).unwrap();
            let by_path = Command::new(&exe).arg("--list").output();
            assert!(by_path.is_err(), "the deleted path should not run");
            let out = self_command()
                .unwrap()
                .arg("--list")
                .stderr(Stdio::null())
                .output()
                .expect("spawn via /proc/self/exe");
            assert!(out.status.success());
            assert!(String::from_utf8_lossy(&out.stdout).contains("self_command_survives"));
            return;
        }
        let exe = std::env::current_exe().unwrap();
        // Same file system as the test binary, so a hard link works (no copy).
        let dir = tempfile::tempdir_in(exe.parent().unwrap()).unwrap();
        let copy = dir.path().join("voxtype-self-exec-test");
        std::fs::hard_link(&exe, &copy).unwrap();
        let status = Command::new(&copy)
            .args([
                "transcribe::subprocess::tests::self_command_survives_a_deleted_binary",
                "--exact",
                "--quiet",
            ])
            .env(SELF_EXEC_HARNESS, "1")
            .status()
            .unwrap();
        assert!(status.success(), "harness failed: {}", status);
        assert!(!copy.exists(), "the harness deleted its own file");
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
