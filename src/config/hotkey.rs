//! Hotkey detection configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::default_true;

/// Hotkey activation mode
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// Hold key to record, release to stop (default)
    #[default]
    PushToTalk,
    /// Press once to start recording, press again to stop
    Toggle,
}

/// Hotkey detection configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    /// Key name (evdev KEY_* constant name, without the KEY_ prefix)
    /// Examples: "SCROLLLOCK", "RIGHTALT", "PAUSE", "F24"
    #[serde(default = "default_hotkey_key")]
    pub key: String,

    /// Optional modifier keys that must also be held
    /// Examples: ["LEFTCTRL"], ["LEFTALT", "LEFTSHIFT"]
    #[serde(default)]
    pub modifiers: Vec<String>,

    /// Activation mode: push_to_talk (hold to record) or toggle (press to start/stop)
    #[serde(default)]
    pub mode: ActivationMode,

    /// Enable built-in hotkey detection (default: true)
    /// Set to false when using compositor keybindings (Hyprland, Sway) instead
    /// When disabled, use `voxtype record start/stop/toggle` to control recording
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional cancel key (evdev KEY_* constant name, without KEY_ prefix)
    /// When pressed, cancels the current recording or transcription
    /// Examples: "ESC", "BACKSPACE", "F12"
    #[serde(default)]
    pub cancel_key: Option<String>,

    /// Optional modifier key for secondary model selection (evdev KEY_* name, without KEY_ prefix)
    /// When held while pressing the hotkey, uses secondary_model instead of the default model
    /// Examples: "LEFTSHIFT", "RIGHTALT", "LEFTCTRL"
    #[serde(default)]
    pub model_modifier: Option<String>,

    /// Optional modifier keys that activate named profiles (evdev KEY_* names, without KEY_ prefix)
    /// When held while pressing the hotkey, activates the named profile for post-processing
    /// Example: { "LEFTSHIFT" = "translate" } activates [profiles.translate] when Shift is held
    #[serde(default)]
    pub profile_modifiers: HashMap<String, String>,

    /// Fire the hotkey on a double tap instead of a single press. A tap is a
    /// press and release of `key` within `double_tap_ms` with no other key
    /// pressed meanwhile, so shortcuts that use the key (Ctrl+C on Right
    /// Ctrl) never count. Two taps within `double_tap_ms` fire one press.
    /// Use with `mode = "toggle"`.
    #[serde(default)]
    pub double_tap: bool,

    /// Longest tap, and longest gap between the two taps, in milliseconds.
    #[serde(default = "default_double_tap_ms")]
    pub double_tap_ms: u64,

    /// A second key whose double tap also starts and stops recording (with
    /// `double_tap`). Each double tap writes which key fired, "main" or
    /// "alt", to `hotkey-last` in the runtime dir, so an output hook can act
    /// on how the recording was stopped (e.g. paste without pressing Enter).
    #[serde(default)]
    pub double_tap_alt_key: Option<String>,

    /// A key whose double tap discards the recording, like `cancel_key`
    /// (with `double_tap`). Unlike a single-press cancel key, a stray press
    /// of the key alone never throws a dictation away.
    #[serde(default)]
    pub double_tap_cancel_key: Option<String>,

    /// Left Ctrl and Right Ctrl held, then Down or Up, switches the vo
    /// popup to its next or previous look while the popup shows. The look
    /// is kept for later dictations. The chord never counts as a tap of
    /// either Ctrl key.
    #[serde(default = "default_true")]
    pub look_chord: bool,

    /// A key whose clean hold while recording marks a spoken correction:
    /// hold it, say the right words, let go, and they replace the words
    /// they sound most like. Taps and double taps of the key keep their
    /// meaning, so it can be `double_tap_alt_key`. Pressing any other key
    /// during the hold drops the correction. Unset (the default) turns
    /// corrections off.
    #[serde(default)]
    pub correction_key: Option<String>,

    /// How long `correction_key` must be held before a correction starts,
    /// in milliseconds. With `double_tap` it is raised to at least
    /// `double_tap_ms` + 150, so a slow tap never starts a correction.
    #[serde(default = "default_correction_hold_ms")]
    pub correction_hold_ms: u64,

    /// A key whose single clean tap while recording pauses the dictation
    /// to fix its text by keyboard in the popup, and resumes it when
    /// tapped again (needs `[osd] keyboard_edit`). Unset, it is
    /// `double_tap_alt_key`. A key that also has a double-tap meaning waits
    /// `double_tap_ms` after the tap to rule out a double tap.
    #[serde(default)]
    pub edit_key: Option<String>,

    /// The edit key's tap counts only when no other key was pressed this
    /// many milliseconds before it, so a Shift pressed while typing does not
    /// pause the dictation.
    #[serde(default = "default_edit_quiet_ms")]
    pub edit_quiet_ms: u64,
}

impl HotkeyConfig {
    /// The edit key in effect: `edit_key`, else `double_tap_alt_key`.
    pub fn edit_key(&self) -> Option<&str> {
        self.edit_key
            .as_deref()
            .or(self.double_tap_alt_key.as_deref())
    }
}

fn default_double_tap_ms() -> u64 {
    300
}

fn default_correction_hold_ms() -> u64 {
    450
}

fn default_edit_quiet_ms() -> u64 {
    150
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: default_hotkey_key(),
            modifiers: Vec::new(),
            mode: ActivationMode::default(),
            enabled: true,
            cancel_key: None,
            model_modifier: None,
            profile_modifiers: HashMap::new(),
            double_tap: false,
            double_tap_ms: default_double_tap_ms(),
            double_tap_alt_key: None,
            double_tap_cancel_key: None,
            look_chord: true,
            correction_key: None,
            correction_hold_ms: default_correction_hold_ms(),
            edit_key: None,
            edit_quiet_ms: default_edit_quiet_ms(),
        }
    }
}

pub(super) fn default_hotkey_key() -> String {
    #[cfg(target_os = "macos")]
    {
        "FN".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "SCROLLLOCK".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn edit_key_defaults_to_the_alt_double_tap_key() {
        let old: HotkeyConfig = toml::from_str(
            "key = \"RIGHTCTRL\"\ndouble_tap = true\ndouble_tap_alt_key = \"RIGHTSHIFT\"",
        )
        .unwrap();
        assert_eq!(old.edit_key, None);
        assert_eq!(old.edit_key(), Some("RIGHTSHIFT"));
        assert_eq!(old.edit_quiet_ms, 150);
        let set: HotkeyConfig = toml::from_str(
            "double_tap_alt_key = \"RIGHTSHIFT\"\nedit_key = \"F9\"\nedit_quiet_ms = 200",
        )
        .unwrap();
        assert_eq!(set.edit_key(), Some("F9"));
        assert_eq!(set.edit_quiet_ms, 200);
        assert_eq!(HotkeyConfig::default().edit_key(), None);
    }

    #[test]
    fn test_parse_hotkey_disabled_without_key() {
        // Regression test for GitHub issue #17
        // When hotkey is disabled, the key field should not be required
        let toml_str = r#"
            [hotkey]
            enabled = false

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.hotkey.enabled);
        assert_eq!(config.hotkey.key, default_hotkey_key()); // platform default
    }

    #[test]
    fn test_parse_toggle_mode() {
        let toml_str = r#"
            [hotkey]
            key = "F13"
            mode = "toggle"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [audio.feedback]
            enabled = true
            theme = "subtle"
            volume = 0.5

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.key, "F13");
        assert_eq!(config.hotkey.mode, ActivationMode::Toggle);
        assert!(config.audio.feedback.enabled);
        assert_eq!(config.audio.feedback.theme, "subtle");
        assert_eq!(config.audio.feedback.volume, 0.5);
    }

    #[test]
    fn test_parse_profile_modifiers() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [hotkey.profile_modifiers]
            LEFTSHIFT = "translate"
            RIGHTALT = "formal"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"

            [profiles.translate]
            post_process_command = "translate.sh"

            [profiles.formal]
            post_process_command = "formal.sh"
            post_process_timeout_ms = 15000
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hotkey.profile_modifiers.len(), 2);
        assert_eq!(
            config.hotkey.profile_modifiers.get("LEFTSHIFT").unwrap(),
            "translate"
        );
        assert_eq!(
            config.hotkey.profile_modifiers.get("RIGHTALT").unwrap(),
            "formal"
        );
        assert!(config.get_profile("translate").is_some());
        assert!(config.get_profile("formal").is_some());
        assert_eq!(
            config
                .get_profile("translate")
                .unwrap()
                .post_process_command
                .as_deref(),
            Some("translate.sh")
        );
    }

    #[test]
    fn test_profile_modifiers_default_empty() {
        let toml_str = r#"
            [hotkey]
            key = "SCROLLLOCK"

            [audio]
            device = "default"
            sample_rate = 16000
            max_duration_secs = 60

            [whisper]
            model = "base.en"
            language = "en"

            [output]
            mode = "type"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.hotkey.profile_modifiers.is_empty());
    }
}
