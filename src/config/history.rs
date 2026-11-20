//! `[history]`: keep a record of every dictation.
//!
//! Off unless the user asks for it, so an upgrade never starts writing
//! someone's dictation to disk. Ported from upstream voxtype v1.3.1, without
//! its focused-app settings (`exclude_apps`, `record_app`,
//! `record_window_title`), and with `max_entries` added.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Dictation history configuration.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HistoryConfig {
    /// Record each dictation to `~/.local/share/voxtype/history/`.
    #[serde(default)]
    pub enabled: bool,

    /// Stop recording until resumed, without turning history off. Paused
    /// dictations are never written.
    #[serde(default)]
    pub paused: bool,

    /// Days of history to keep. Older day files are deleted at daemon start
    /// and once a day. 0 keeps history forever.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,

    /// Most dictations to keep. After each dictation the oldest beyond this
    /// many are deleted. 0 keeps any number.
    #[serde(default = "default_max_entries")]
    pub max_entries: u32,
}

fn default_retention_days() -> u32 {
    30
}

fn default_max_entries() -> u32 {
    200
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            paused: false,
            retention_days: default_retention_days(),
            max_entries: default_max_entries(),
        }
    }
}

impl HistoryConfig {
    /// Whether a dictation should be written.
    pub fn recording(&self) -> bool {
        self.enabled && !self.paused
    }
}

/// History settings given on the daemon's command line. Kept apart from the
/// rest of the config because the daemon re-reads `[history]` from disk when
/// it changes (`voxtype history pause` writes it), and a flag must still win
/// over the file after that.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryOverrides {
    pub enabled: Option<bool>,
    pub paused: Option<bool>,
    pub retention_days: Option<u32>,
    pub max_entries: Option<u32>,
}

static CLI_OVERRIDES: OnceLock<HistoryOverrides> = OnceLock::new();

impl HistoryOverrides {
    pub fn apply(&self, config: &mut HistoryConfig) {
        if let Some(v) = self.enabled {
            config.enabled = v;
        }
        if let Some(v) = self.paused {
            config.paused = v;
        }
        if let Some(v) = self.retention_days {
            config.retention_days = v;
        }
        if let Some(v) = self.max_entries {
            config.max_entries = v;
        }
    }

    /// Remember this process's command-line overrides. Only the first call
    /// counts.
    pub fn set_cli(self) {
        let _ = CLI_OVERRIDES.set(self);
    }

    /// The overrides given on this process's command line.
    pub fn cli() -> HistoryOverrides {
        CLI_OVERRIDES.get().cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrap {
        #[serde(default)]
        history: HistoryConfig,
    }

    #[test]
    fn a_missing_table_is_off_with_the_shipped_defaults() {
        let w: Wrap = toml::from_str("").unwrap();
        assert_eq!(w.history, HistoryConfig::default());
        assert!(!w.history.enabled);
        assert!(!w.history.recording());
        assert_eq!(w.history.retention_days, 30);
        assert_eq!(w.history.max_entries, 200);
    }

    #[test]
    fn an_empty_table_is_still_off() {
        let w: Wrap = toml::from_str("[history]\n").unwrap();
        assert!(!w.history.enabled);
    }

    #[test]
    fn paused_stops_recording_without_turning_history_off() {
        let w: Wrap = toml::from_str("[history]\nenabled = true\npaused = true\n").unwrap();
        assert!(w.history.enabled);
        assert!(!w.history.recording());
    }

    #[test]
    fn upstream_keys_this_build_lacks_are_ignored() {
        let w: Wrap = toml::from_str(
            "[history]\nenabled = true\nexclude_apps = [\"Foo\"]\nrecord_app = false\n",
        )
        .unwrap();
        assert!(w.history.enabled);
    }

    #[test]
    fn overrides_win_field_by_field() {
        let mut h = HistoryConfig::default();
        HistoryOverrides {
            enabled: Some(true),
            retention_days: Some(0),
            max_entries: Some(5),
            ..Default::default()
        }
        .apply(&mut h);
        assert!(h.enabled);
        assert!(!h.paused);
        assert_eq!(h.retention_days, 0);
        assert_eq!(h.max_entries, 5);
    }
}
