//! The daemon's side of history: build an entry when a dictation is
//! delivered, or fails to be, and write it in the background.
//!
//! Writing never delays output. The daemon calls [`Recorder::record_delivery`]
//! once the output driver has answered, and the day file is written on a
//! blocking thread.
//!
//! Settings are re-read from the config file whenever it changes, so
//! `voxtype history pause` takes effect on the next dictation without a
//! restart. Daemon flags still win afterwards.
//!
//! Slimmed from upstream voxtype v1.3.1: no focused-app lookup (`app` and
//! `window_title` stay empty), no search index, no privacy lock. A dictation
//! is recorded once, at delivery, instead of being built up from the start
//! of the recording.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{Local, NaiveDate};

use super::entry::{HistoryEntry, Outcome, OutputInfo};
use super::store::HistoryStore;
use crate::config::{Config, HistoryConfig, HistoryOverrides};

/// One finished dictation, as the daemon saw it at delivery.
pub struct Delivery<'a> {
    /// The final text: what was delivered, or what could not be.
    pub text: &'a str,
    /// What the engine returned, when the daemon still has it.
    pub raw_text: Option<&'a str>,
    /// The driver that took the text, or why delivery failed.
    pub result: Result<&'a str, String>,
    /// `type`, `paste`, `clipboard` or `file`.
    pub mode: &'a str,
    /// A live or streamed dictation.
    pub streaming: bool,
    /// Enter was pressed after the text.
    pub auto_submit: bool,
    /// How long the recording was, when known.
    pub audio_ms: Option<u64>,
    /// How long ago the recording stopped, when known.
    pub since_stop_ms: Option<u64>,
}

impl<'a> Delivery<'a> {
    /// A dictation with `text`, delivered with `result` in `mode`; the other
    /// fields unknown.
    pub fn new(text: &'a str, result: Result<&'a str, String>, mode: &'a str) -> Self {
        Self {
            text,
            raw_text: None,
            result,
            mode,
            streaming: false,
            auto_submit: false,
            audio_ms: None,
            since_stop_ms: None,
        }
    }

    /// The history entry for this dictation, ending now.
    pub fn entry(&self, config: &Config) -> HistoryEntry {
        let now = Local::now();
        let back = self.since_stop_ms.unwrap_or(0) + self.audio_ms.unwrap_or(0);
        let start = now - chrono::Duration::milliseconds(back as i64);
        let mut entry = HistoryEntry::new(start);
        entry.ts_end = now.fixed_offset();
        let raw = self.raw_text.unwrap_or(self.text);
        entry.raw_text = raw.to_string();
        entry.text = self.text.to_string();
        entry.edits = super::entry::edits_between(raw, self.text);
        entry.engine = config.engine.name().to_string();
        entry.model = config.model_name().to_string();
        let (variant, backend) = binary_variant_and_backend();
        entry.variant = variant;
        entry.backend = backend;
        entry.audio_ms = self.audio_ms.unwrap_or(0);
        entry.streaming = self.streaming;
        entry.auto_submit = self.auto_submit;
        let mut output = OutputInfo {
            mode: self.mode.to_string(),
            ..Default::default()
        };
        match &self.result {
            Ok(driver) => {
                output.driver = Some(driver.to_string());
                entry.set_outcome(Outcome::Delivered, None);
            }
            Err(error) => entry.set_outcome(Outcome::OutputFailed, Some(error.clone())),
        }
        entry.output = Some(output);
        entry
    }
}

/// What the daemon keeps between dictations.
#[derive(Clone)]
pub struct Recorder {
    /// `--config`, or `None` for the usual lookup.
    config_path: Option<PathBuf>,
    settings: HistoryConfig,
    /// The config file as last read: modification time and size.
    seen: Option<(SystemTime, u64)>,
    store_dir: PathBuf,
    last_maintenance: Option<NaiveDate>,
}

impl Recorder {
    pub fn new(config: &Config, config_path: Option<PathBuf>) -> Self {
        let mut recorder = Self {
            config_path,
            settings: config.history.clone(),
            seen: None,
            store_dir: HistoryStore::default_dir(),
            last_maintenance: None,
        };
        recorder.seen = recorder.file_stamp();
        recorder
    }

    /// A recorder writing to an explicit place, for tests.
    pub fn with_paths(settings: HistoryConfig, store_dir: PathBuf) -> Self {
        Self {
            config_path: None,
            settings,
            seen: None,
            store_dir,
            last_maintenance: None,
        }
    }

    fn file_path(&self) -> Option<PathBuf> {
        // Unit tests build daemons without a config path; they must never
        // pick up the developer's own config and start writing history.
        if cfg!(test) && self.config_path.is_none() {
            return None;
        }
        self.config_path
            .clone()
            .or_else(Config::resolve_existing_path)
    }

    fn file_stamp(&self) -> Option<(SystemTime, u64)> {
        let meta = std::fs::metadata(self.file_path()?).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// Re-read `[history]` if the config file changed since it was last
    /// read. A file that no longer parses keeps the settings in force.
    pub fn refresh(&mut self) -> &HistoryConfig {
        let stamp = self.file_stamp();
        if stamp.is_some() && stamp != self.seen {
            self.seen = stamp;
            match crate::config::load_config(self.file_path().as_deref()) {
                Ok(config) => {
                    let mut settings = config.history;
                    HistoryOverrides::cli().apply(&mut settings);
                    if settings != self.settings {
                        tracing::info!("History settings changed: {}", describe(&settings));
                    }
                    self.settings = settings;
                }
                Err(e) => tracing::warn!("Could not re-read [history]: {}", e),
            }
        }
        &self.settings
    }

    pub fn settings(&self) -> &HistoryConfig {
        &self.settings
    }

    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    /// Record a delivered or failed dictation, when history is on and not
    /// paused. Returns the background write so tests can wait for it.
    pub fn record_delivery(
        &mut self,
        config: &Config,
        delivery: &Delivery<'_>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.record(delivery.entry(config))
    }

    /// Write `entry` in the background, when history is on and not paused.
    pub fn record(&mut self, mut entry: HistoryEntry) -> Option<tokio::task::JoinHandle<()>> {
        if !self.refresh().recording() {
            return None;
        }
        let rt = tokio::runtime::Handle::try_current().ok()?;
        let mut guard = self.clone();
        let today = Local::now().date_naive();
        let maintain = self.last_maintenance != Some(today);
        self.last_maintenance = Some(today);
        Some(rt.spawn_blocking(
            move || match write(&mut guard, &mut entry, maintain, today) {
                Ok(true) => tracing::debug!(
                    "History: recorded {} ({})",
                    entry.id,
                    entry.outcome.as_str()
                ),
                Ok(false) => tracing::debug!("History is off or paused; dictation not recorded"),
                Err(e) => tracing::warn!("Could not record dictation history: {}", e),
            },
        ))
    }

    /// Retention, compression and the entry cap, in the background. The
    /// daemon runs this at start; [`Recorder::record`] runs it after the
    /// first dictation of each day.
    pub fn maintain(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let settings = self.refresh().clone();
        if !settings.enabled {
            return None;
        }
        let rt = tokio::runtime::Handle::try_current().ok()?;
        let dir = self.store_dir.clone();
        let today = Local::now().date_naive();
        self.last_maintenance = Some(today);
        Some(rt.spawn_blocking(move || maintenance(&dir, &settings, today)))
    }
}

/// Append `entry` if the settings, re-read under the store lock, still allow
/// it. Then apply the cap, and once a day the retention.
fn write(
    guard: &mut Recorder,
    entry: &mut HistoryEntry,
    maintain: bool,
    today: NaiveDate,
) -> Result<bool, String> {
    let store = HistoryStore::at(guard.store_dir.clone());
    let mut settings = None;
    let written = store
        .append_prepared(entry, |_| {
            // Paused or turned off since the dictation ended: write nothing.
            guard.seen = None;
            let current = guard.refresh().clone();
            if !current.recording() {
                return Ok(None);
            }
            settings = Some(current);
            Ok(Some(()))
        })
        .map_err(|e| e.to_string())?;
    match settings {
        Some(settings) if written => {
            if maintain {
                maintenance(&guard.store_dir, &settings, today);
            } else {
                cap(&store, &settings);
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn cap(store: &HistoryStore, settings: &HistoryConfig) {
    match store.cap(settings.max_entries as usize) {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            "History: removed {} dictation(s) beyond the {} kept",
            n,
            settings.max_entries
        ),
        Err(e) => tracing::warn!("History entry cap failed: {}", e),
    }
}

/// Retention, compression and the entry cap.
pub fn maintenance(store_dir: &Path, settings: &HistoryConfig, today: NaiveDate) {
    if !store_dir.exists() {
        return;
    }
    let store = match HistoryStore::open(store_dir) {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!("History maintenance skipped: {}", e);
            return;
        }
    };
    match store.prune(settings.retention_days, today) {
        Ok(removed) if !removed.is_empty() => tracing::info!(
            "History: removed {} day(s) older than {} days",
            removed.len(),
            settings.retention_days
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("History retention failed: {}", e),
    }
    if let Err(e) = store.compress_old(today) {
        tracing::warn!("Compressing old history failed: {}", e);
    }
    cap(&store, settings);
}

/// The running binary's variant (`avx2`, `onnx-migraphx`, ...), from its
/// file name, and the GPU backend that variant was built for (`cpu` when
/// none). This is what the binary can use, not proof a GPU ran the model.
pub fn binary_variant_and_backend() -> (Option<String>, Option<String>) {
    static CACHED: std::sync::OnceLock<(Option<String>, Option<String>)> =
        std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| {
            let variant = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .and_then(|name| name.strip_prefix("voxtype-").map(str::to_string))
                .filter(|v| !v.is_empty());
            (
                variant.clone(),
                Some(backend_for(variant.as_deref()).to_string()),
            )
        })
        .clone()
}

fn backend_for(variant: Option<&str>) -> &'static str {
    let v = variant.unwrap_or_default();
    if v.contains("cuda") || cfg!(feature = "gpu-cuda") {
        "cuda"
    } else if v.contains("migraphx") {
        "migraphx"
    } else if v.contains("rocm") || cfg!(feature = "gpu-hipblas") {
        "rocm"
    } else if v.contains("vulkan") || cfg!(feature = "gpu-vulkan") {
        "vulkan"
    } else if cfg!(feature = "gpu-metal") {
        "metal"
    } else {
        "cpu"
    }
}

/// One line of settings, for the log. Never any dictated text.
fn describe(s: &HistoryConfig) -> String {
    format!(
        "{}{}, keep {}, {}",
        if s.enabled { "on" } else { "off" },
        if s.paused { " (paused)" } else { "" },
        if s.retention_days == 0 {
            "forever".to_string()
        } else {
            format!("{} days", s.retention_days)
        },
        if s.max_entries == 0 {
            "no entry limit".to_string()
        } else {
            format!("at most {} dictations", s.max_entries)
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder(settings: HistoryConfig) -> (tempfile::TempDir, Recorder) {
        let t = tempfile::tempdir().unwrap();
        let r = Recorder::with_paths(settings, t.path().join("history"));
        (t, r)
    }

    fn on() -> HistoryConfig {
        HistoryConfig {
            enabled: true,
            ..Default::default()
        }
    }

    fn delivery<'a>(text: &'a str, result: Result<&'a str, String>) -> Delivery<'a> {
        Delivery {
            text,
            raw_text: None,
            result,
            mode: "type",
            streaming: false,
            auto_submit: false,
            audio_ms: Some(1500),
            since_stop_ms: Some(200),
        }
    }

    #[tokio::test]
    async fn off_or_paused_writes_nothing() {
        let config = Config::default();
        let (t, mut r) = recorder(HistoryConfig::default());
        let d = delivery("The quick brown fox.", Ok("wtype"));
        assert!(r.record_delivery(&config, &d).is_none());
        let (t2, mut r2) = recorder(HistoryConfig {
            paused: true,
            ..on()
        });
        assert!(r2.record_delivery(&config, &d).is_none());
        assert!(!t.path().join("history").exists());
        assert!(!t2.path().join("history").exists());
    }

    #[tokio::test]
    async fn a_delivered_dictation_is_written() {
        let (t, mut r) = recorder(on());
        let d = Delivery {
            raw_text: Some("the quick brown fox"),
            ..delivery("The quick brown fox.", Ok("wtype"))
        };
        r.record_delivery(&Config::default(), &d)
            .unwrap()
            .await
            .unwrap();

        let entries = HistoryStore::at(t.path().join("history")).all().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.text, "The quick brown fox.");
        assert_eq!(e.raw_text, "the quick brown fox");
        assert!(!e.edits.is_empty());
        assert!(e.delivered);
        assert_eq!(e.outcome, Outcome::Delivered);
        assert_eq!(e.audio_ms, 1500);
        let output = e.output.as_ref().unwrap();
        assert_eq!(output.mode, "type");
        assert_eq!(output.driver.as_deref(), Some("wtype"));
        assert!(e.ts_end > e.ts_start);
    }

    #[tokio::test]
    async fn a_failed_delivery_is_recorded_with_its_text() {
        let (t, mut r) = recorder(on());
        let d = delivery("Jumps over the lazy dog.", Err("no driver worked".into()));
        r.record_delivery(&Config::default(), &d)
            .unwrap()
            .await
            .unwrap();

        let entries = HistoryStore::at(t.path().join("history")).all().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.text, "Jumps over the lazy dog.");
        assert_eq!(e.best_text(), "Jumps over the lazy dog.");
        assert!(!e.delivered);
        assert_eq!(e.outcome, Outcome::OutputFailed);
        assert_eq!(e.error.as_deref(), Some("no driver worked"));
        assert_eq!(e.output.as_ref().unwrap().driver, None);
    }

    #[tokio::test]
    async fn the_cap_applies_after_each_dictation() {
        let (t, mut r) = recorder(HistoryConfig {
            max_entries: 2,
            ..on()
        });
        let config = Config::default();
        for text in [
            "One made-up line.",
            "Two made-up lines.",
            "Three made-up lines.",
        ] {
            r.record_delivery(&config, &delivery(text, Ok("wtype")))
                .unwrap()
                .await
                .unwrap();
        }
        let store = HistoryStore::at(t.path().join("history"));
        let texts: Vec<_> = store.all().unwrap().into_iter().map(|e| e.text).collect();
        assert_eq!(texts.len(), 2);
        assert!(
            !texts.contains(&"One made-up line.".to_string()),
            "{texts:?}"
        );
    }

    #[tokio::test]
    async fn pausing_in_the_config_file_applies_to_the_next_dictation() {
        let t = tempfile::tempdir().unwrap();
        let cfg_path = t.path().join("config.toml");
        std::fs::write(&cfg_path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&cfg_path)).unwrap();
        let mut r = Recorder::new(&config, Some(cfg_path.clone()));
        r.store_dir = t.path().join("history");
        let d = delivery("The quick brown fox.", Ok("wtype"));
        r.record_delivery(&config, &d).unwrap().await.unwrap();

        // A different size guarantees the change is seen even within one
        // mtime tick.
        std::fs::write(&cfg_path, "[history]\nenabled = true\npaused = true\n").unwrap();
        assert!(r.record_delivery(&config, &d).is_none());
        std::fs::write(
            &cfg_path,
            "[history]\nenabled = true\npaused = false # resumed\n",
        )
        .unwrap();
        r.record_delivery(&config, &d).unwrap().await.unwrap();
        assert_eq!(HistoryStore::at(&r.store_dir).all().unwrap().len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pausing_before_the_queued_writer_runs_discards_the_entry() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(&path, "[history]\nenabled = true\n").unwrap();
        let config = crate::config::load_config(Some(&path)).unwrap();
        let mut r = Recorder::new(&config, Some(path.clone()));
        r.store_dir = t.path().join("history");
        // Hold the store lock so the writer waits until the pause is saved.
        HistoryStore::open(&r.store_dir).unwrap();
        let held = super::super::store::DirLock::take(&r.store_dir).unwrap();
        let writer = r
            .record_delivery(&config, &delivery("Synthetic words.", Ok("wtype")))
            .unwrap();
        std::fs::write(&path, "[history]\nenabled = true\npaused = true\n").unwrap();
        drop(held);
        writer.await.unwrap();
        assert!(HistoryStore::at(&r.store_dir).all().unwrap().is_empty());
    }

    #[test]
    fn backends_follow_the_variant_name() {
        if cfg!(any(
            feature = "gpu-cuda",
            feature = "gpu-hipblas",
            feature = "gpu-vulkan",
            feature = "gpu-metal"
        )) {
            return;
        }
        assert_eq!(backend_for(Some("onnx-cuda-13")), "cuda");
        assert_eq!(backend_for(Some("onnx-migraphx")), "migraphx");
        assert_eq!(backend_for(Some("vulkan")), "vulkan");
        assert_eq!(backend_for(Some("avx2")), "cpu");
        assert_eq!(backend_for(None), "cpu");
    }

    #[test]
    fn describe_never_mentions_text() {
        assert_eq!(describe(&on()), "on, keep 30 days, at most 200 dictations");
        assert_eq!(
            describe(&HistoryConfig {
                paused: true,
                retention_days: 0,
                max_entries: 0,
                ..on()
            }),
            "on (paused), keep forever, no entry limit"
        );
    }
}
