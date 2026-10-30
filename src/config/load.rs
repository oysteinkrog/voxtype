use super::parse::{merge_toml_values, parse_config_salvaging};
use super::{
    Config, LanguageConfig, OpenVinoConfig, OutputMode, SonioxConfig, TranscriptionEngine,
};
use crate::error::VoxtypeError;
use std::path::{Path, PathBuf};

/// Parse a boolean from an environment variable value.
/// Only "1" and "true" (case-insensitive) are truthy; everything else is falsy.
fn parse_bool_env(val: &str) -> bool {
    val == "1" || val.eq_ignore_ascii_case("true")
}

/// Top-level tables that voxtype only reads nested elsewhere, as
/// `(name, where it belongs)`. Serde ignores them silently, which is how a
/// `[post_process]` meant for `[output.post_process]` never ran.
fn misplaced_sections(contents: &str) -> Vec<(String, String)> {
    let Ok(table) = contents.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .iter()
        .filter(|(_, value)| value.is_table())
        .filter_map(|(name, _)| {
            super::schema::misplaced_section_home(name).map(|home| (name.clone(), home))
        })
        .collect()
}

/// Add the `"from" = "to"` pairs in `file` to `replacements`, keeping any
/// entry already there. A missing or bad file is logged and skipped, so a
/// word list that is not synced to this machine never stops the daemon.
fn merge_replacements_file(
    replacements: &mut std::collections::HashMap<String, String>,
    file: &str,
) {
    let path = match (file.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(file),
    };
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| {
            toml::from_str::<std::collections::HashMap<String, String>>(&s)
                .map_err(|e| e.to_string())
        });
    match parsed {
        Ok(extra) => {
            let n = extra.len();
            for (from, to) in extra {
                replacements.entry(from).or_insert(to);
            }
            tracing::debug!("Read {} word replacements from {:?}", n, path);
        }
        Err(e) => tracing::warn!("Skipping text.replacements_file {:?}: {}", path, e),
    }
}

/// The overlay files for the config file at `config_path`: the `*.toml`
/// files in the `config.d` directory next to it, in name order. None when
/// there is no such directory.
fn overlay_paths(config_path: &Path) -> Vec<PathBuf> {
    let Some(dir) = config_path.parent().map(|d| d.join("config.d")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
        .collect();
    paths.sort();
    paths
}

/// `contents` with each overlay merged over it in order: tables merge key by
/// key, any other value replaces the one before it. Lets one config file be
/// shared between machines, with a small file per machine for what differs.
/// An overlay that cannot be read or parsed is named in the log and skipped.
/// When `contents` itself does not parse it is returned unchanged, so its
/// own error is the one reported.
fn apply_overlays(contents: &str, overlays: &[PathBuf]) -> String {
    let Ok(mut merged) = contents.parse::<toml::Table>().map(toml::Value::Table) else {
        return contents.to_string();
    };
    for path in overlays {
        let overlay = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| s.parse::<toml::Table>().map_err(|e| e.to_string()));
        match overlay {
            Ok(table) => {
                tracing::debug!("Applying config overlay {:?}", path);
                merge_toml_values(&mut merged, toml::Value::Table(table));
            }
            Err(e) => tracing::warn!("Skipping config overlay {:?}: {}", path, e),
        }
    }
    toml::to_string(&merged).unwrap_or_else(|_| contents.to_string())
}

pub fn load_config(path: Option<&Path>) -> Result<Config, VoxtypeError> {
    load_config_with_env(path, &|key| std::env::var(key))
}

/// The `VOXTYPE_*` overrides: a variable's value, as `std::env::var` gives it.
type Env<'a> = &'a dyn Fn(&str) -> Result<String, std::env::VarError>;

/// [`load_config`] with the environment overrides read from `env`, so tests
/// need not change the process environment that other tests read at the
/// same time.
fn load_config_with_env(path: Option<&Path>, env: Env) -> Result<Config, VoxtypeError> {
    // Start with defaults
    let mut config = Config::default();

    // Determine config file path. If --config wasn't passed, walk the
    // documented lookup chain: user config -> /etc/voxtype/config.toml.
    let config_path = match path {
        Some(p) => Some(PathBuf::from(p)),
        None => Config::resolve_existing_path(),
    };

    // Load from file if it exists. Layer the user's TOML over a default-rendered
    // Config so partial sections inherit defaults for fields the user didn't set.
    // Without this, a user config containing only `[hotkey]` or `[audio.feedback]`
    // would fail with "missing field 'audio'" / "missing field 'device'", because
    // serde's per-field defaults only fill in what the user omitted at the *field*
    // level, not what they omitted at a parent-section level (#421).
    //
    // Overlays in config.d/ next to the file (or next to where the user
    // config would be) are merged over it first.
    let overlays = config_path
        .clone()
        .or_else(Config::default_path)
        .map(|p| overlay_paths(&p))
        .unwrap_or_default();
    let config_path = match config_path {
        None if !overlays.is_empty() => Config::default_path(),
        found => found,
    };
    if let Some(ref path) = config_path {
        if path.exists() || !overlays.is_empty() {
            tracing::debug!("Loading config from {:?}", path);
            let contents = if path.exists() {
                std::fs::read_to_string(path)
                    .map_err(|e| VoxtypeError::Config(format!("Failed to read config: {}", e)))?
            } else {
                String::new()
            };
            let contents = if overlays.is_empty() {
                contents
            } else {
                apply_overlays(&contents, &overlays)
            };

            // Salvage per section rather than refusing to start: one bad
            // value used to cost the user every other setting they had
            // (#646). A TOML syntax error is still fatal, because there is
            // nothing to salvage from a file we cannot parse at all.
            let (parsed, rejected) = parse_config_salvaging(&contents)
                .map_err(|e| VoxtypeError::Config(format!("Invalid config: {}", e)))?;
            config = parsed;

            for key in &rejected {
                // Names only. These sections hold API keys for the remote and
                // Soniox backends, so the values must never reach the log.
                tracing::warn!(
                    "Config section '{}' could not be read and is using defaults. \
                     Check it against `voxtype config schema`.",
                    key
                );
            }
            if !rejected.is_empty() {
                tracing::warn!(
                    "{} config section(s) were skipped: {}",
                    rejected.len(),
                    rejected.join(", ")
                );
            }
            for (name, home) in misplaced_sections(&contents) {
                tracing::warn!(
                    "Config section [{}] is ignored at the top level. \
                     Voxtype reads it as {}; move it there.",
                    name,
                    home
                );
            }
        } else {
            tracing::debug!("Config file not found at {:?}, using defaults", path);
        }
    } else {
        tracing::debug!("No config file found at user or system path, using built-in defaults");
    }

    if let Some(file) = config.text.replacements_file.clone() {
        merge_replacements_file(&mut config.text.replacements, &file);
    }

    // Override from environment variables
    // Hotkey
    if let Ok(key) = env("VOXTYPE_HOTKEY") {
        config.hotkey.key = key;
    }
    if let Ok(val) = env("VOXTYPE_HOTKEY_ENABLED") {
        config.hotkey.enabled = parse_bool_env(&val);
    }
    if let Ok(key) = env("VOXTYPE_CANCEL_KEY") {
        config.hotkey.cancel_key = Some(key);
    }
    if let Ok(key) = env("VOXTYPE_CORRECTION_KEY") {
        config.hotkey.correction_key = Some(key);
    }
    if let Ok(val) = env("VOXTYPE_CORRECTION_HOLD_MS") {
        if let Ok(ms) = val.parse::<u64>() {
            config.hotkey.correction_hold_ms = ms;
        }
    }
    if let Ok(key) = env("VOXTYPE_EDIT_KEY") {
        config.hotkey.edit_key = Some(key);
    }
    if let Ok(val) = env("VOXTYPE_EDIT_QUIET_MS") {
        if let Ok(ms) = val.parse::<u64>() {
            config.hotkey.edit_quiet_ms = ms;
        }
    }
    if let Ok(val) = env("VOXTYPE_OSD_KEYBOARD_EDIT") {
        config.osd.keyboard_edit = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_OSD_DOUBT_BELOW") {
        if let Ok(v) = val.parse::<f32>() {
            config.osd.doubt_below = v;
        }
    }

    // Whisper / engine
    if let Ok(model) = env("VOXTYPE_MODEL") {
        config.whisper.model = model;
    }
    if let Ok(engine) = env("VOXTYPE_ENGINE") {
        match engine.parse::<TranscriptionEngine>() {
            Ok(e) => config.engine = e,
            Err(_) => tracing::warn!("Unknown VOXTYPE_ENGINE value: {}", engine),
        }
    }
    if let Ok(lang) = env("VOXTYPE_LANGUAGE") {
        config.whisper.language = LanguageConfig::from_comma_separated(&lang);
    }
    if let Ok(val) = env("VOXTYPE_TRANSLATE") {
        config.whisper.translate = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_THREADS") {
        if let Ok(n) = val.parse::<usize>() {
            config.whisper.threads = Some(n);
        }
    }
    if let Ok(val) = env("VOXTYPE_GPU_ISOLATION") {
        config.whisper.gpu_isolation = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_GPU_DEVICE") {
        if let Ok(n) = val.parse::<i32>() {
            config.whisper.gpu_device = Some(n);
        }
    }
    if let Ok(val) = env("VOXTYPE_FLASH_ATTENTION") {
        config.whisper.flash_attention = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_ON_DEMAND_LOADING") {
        config.whisper.on_demand_loading = parse_bool_env(&val);
    }
    if let Ok(dir) = env("VOXTYPE_OPENVINO_DIR") {
        config
            .openvino
            .get_or_insert_with(OpenVinoConfig::default)
            .openvino_dir = Some(dir);
    }

    // Audio
    if let Ok(device) = env("VOXTYPE_AUDIO_DEVICE") {
        config.audio.device = device;
    }
    if let Ok(val) = env("VOXTYPE_MAX_DURATION_SECS") {
        if let Ok(n) = val.parse::<u32>() {
            config.audio.max_duration_secs = n;
        }
    }
    if let Ok(val) = env("VOXTYPE_AUDIO_FEEDBACK") {
        config.audio.feedback.enabled = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_PAUSE_MEDIA") {
        config.audio.pause_media = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_DUCK_MEDIA") {
        config.audio.duck_media = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_DUCK_MEDIA_VOLUME_PERCENT") {
        if let Ok(n) = val.parse::<u8>() {
            config.audio.duck_media_volume_percent = n.min(150);
        }
    }
    if let Ok(val) = env("VOXTYPE_DUCK_MEDIA_FADE_MS") {
        if let Ok(n) = val.parse::<u32>() {
            config.audio.duck_media_fade_ms = n;
        }
    }

    // Output
    if let Ok(mode) = env("VOXTYPE_OUTPUT_MODE") {
        config.output.mode = match mode.to_lowercase().as_str() {
            "clipboard" => OutputMode::Clipboard,
            "paste" => OutputMode::Paste,
            "file" => OutputMode::File,
            _ => OutputMode::Type,
        };
    }
    if let Ok(append_text) = env("VOXTYPE_APPEND_TEXT") {
        config.output.append_text = Some(append_text);
    }
    if env("VOXTYPE_WTYPE_SHIFT_PREFIX")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        config.output.wtype_shift_prefix = true;
    }
    if let Ok(val) = env("VOXTYPE_AUTO_SUBMIT") {
        config.output.auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_SHIFT_ENTER_NEWLINES") {
        config.output.shift_enter_newlines = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_PRE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.pre_type_delay_ms = n;
        }
    }
    if let Ok(val) = env("VOXTYPE_TYPE_DELAY") {
        if let Ok(n) = val.parse::<u32>() {
            config.output.type_delay_ms = n;
        }
    }
    if let Ok(val) = env("VOXTYPE_FALLBACK_TO_CLIPBOARD") {
        config.output.fallback_to_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_SPOKEN_PUNCTUATION") {
        config.text.spoken_punctuation = parse_bool_env(&val);
    }
    if let Ok(keys) = env("VOXTYPE_PASTE_KEYS") {
        config.output.paste_keys = Some(keys);
    }
    if let Ok(layout) = env("VOXTYPE_DOTOOL_XKB_LAYOUT") {
        config.output.dotool_xkb_layout = Some(layout);
    }
    if let Ok(variant) = env("VOXTYPE_DOTOOL_XKB_VARIANT") {
        config.output.dotool_xkb_variant = Some(variant);
    }
    if let Ok(layout) = env("VOXTYPE_EITYPE_XKB_LAYOUT") {
        config.output.eitype_xkb_layout = Some(layout);
    }
    if let Ok(variant) = env("VOXTYPE_EITYPE_XKB_VARIANT") {
        config.output.eitype_xkb_variant = Some(variant);
    }

    // Remote whisper
    if let Ok(endpoint) = env("VOXTYPE_REMOTE_ENDPOINT") {
        config.whisper.remote_endpoint = Some(endpoint);
    }
    if let Ok(key) = env("VOXTYPE_WHISPER_API_KEY") {
        config.whisper.remote_api_key = Some(key);
    }

    // Soniox
    if let Ok(key) = env("SONIOX_API_KEY") {
        config
            .soniox
            .get_or_insert_with(SonioxConfig::default)
            .api_key = Some(key);
    }
    if let Ok(val) = env("VOXTYPE_RESTORE_CLIPBOARD") {
        config.output.restore_clipboard = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_RESTORE_CLIPBOARD_DELAY_MS") {
        if let Ok(ms) = val.parse::<u32>() {
            config.output.restore_clipboard_delay_ms = ms;
        }
    }
    if let Ok(val) = env("VOXTYPE_SMART_AUTO_SUBMIT") {
        config.text.smart_auto_submit = parse_bool_env(&val);
    }
    if let Ok(val) = env("VOXTYPE_FILTER_FILLERS") {
        config.text.filter_filler_words = parse_bool_env(&val);
    }

    Ok(config)
}

/// Save configuration to file
#[allow(dead_code)]
pub fn save_config(config: &Config, path: &Path) -> Result<(), VoxtypeError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| VoxtypeError::Config(format!("Failed to create config dir: {}", e)))?;
    }

    let contents = toml::to_string_pretty(config)
        .map_err(|e| VoxtypeError::Config(format!("Failed to serialize config: {}", e)))?;

    std::fs::write(path, contents)
        .map_err(|e| VoxtypeError::Config(format!("Failed to write config: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {

    /// An environment with only `vars` set. Tests load through it instead
    /// of setting process variables, which other tests running at the same
    /// time would read.
    fn env_of(
        vars: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Result<String, std::env::VarError> {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    /// [`load_config`] with no `VOXTYPE_*` variables set.
    fn load_alone(path: &Path) -> Result<Config, VoxtypeError> {
        load_config_with_env(Some(path), &env_of(&[]))
    }

    #[test]
    fn keyboard_edit_env_overrides() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[hotkey]\nkey = \"RIGHTCTRL\"\n").unwrap();
        let env = env_of(&[
            ("VOXTYPE_EDIT_KEY", "F9"),
            ("VOXTYPE_EDIT_QUIET_MS", "220"),
            ("VOXTYPE_OSD_KEYBOARD_EDIT", "true"),
            ("VOXTYPE_OSD_DOUBT_BELOW", "0.3"),
        ]);
        let config = load_config_with_env(Some(&path), &env).unwrap();
        assert_eq!(config.hotkey.edit_key(), Some("F9"));
        assert_eq!(config.hotkey.edit_quiet_ms, 220);
        assert!(config.osd.keyboard_edit);
        assert_eq!(config.osd.doubt_below, 0.3);
    }

    use super::*;

    #[test]
    fn replacements_file_adds_entries_inline_ones_win() {
        let dir = tempfile::tempdir().unwrap();
        let words = dir.path().join("words.toml");
        std::fs::write(&words, "\"nos\" = \"NAS\"\n\"vox type\" = \"from file\"\n").unwrap();
        let mut map = std::collections::HashMap::new();
        map.insert("vox type".to_string(), "voxtype".to_string());
        merge_replacements_file(&mut map, words.to_str().unwrap());
        assert_eq!(map.get("nos").map(String::as_str), Some("NAS"));
        assert_eq!(map.get("vox type").map(String::as_str), Some("voxtype"));

        // A missing file leaves the map as it was.
        merge_replacements_file(&mut map, dir.path().join("none.toml").to_str().unwrap());
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn top_level_post_process_is_flagged_with_its_real_home() {
        let config = "[output]\nmode = \"type\"\n\n[post_process]\ncommand = \"sed 's/uh, //g'\"\n";
        assert_eq!(
            misplaced_sections(config),
            vec![(
                "post_process".to_string(),
                "[output.post_process]".to_string()
            )]
        );
    }

    #[test]
    fn real_and_unknown_sections_are_not_flagged() {
        // [audio] is a real top-level section even though [meeting.audio]
        // also exists; an unknown section stays silently ignored as before.
        let config = "[audio]\ndevice = \"default\"\n\n[output.post_process]\ncommand = \"cat\"\n\n[meeting.audio]\nloopback_device = \"auto\"\n\n[some_removed_section]\nx = 1\n";
        assert!(misplaced_sections(config).is_empty());
    }

    #[test]
    fn misplaced_nested_sections_name_their_parent() {
        assert_eq!(
            super::super::schema::misplaced_section_home("notification").as_deref(),
            Some("[output.notification]")
        );
        assert_eq!(
            super::super::schema::misplaced_section_home("whisper"),
            None
        );
    }

    #[test]
    fn test_load_config_explicit_path() {
        // Explicit --config should always be used regardless of fallback.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
                [hotkey]
                key = "F12"

                [audio]
                device = "default"
                sample_rate = 16000
                max_duration_secs = 30

                [whisper]
                model = "tiny.en"
                language = "en"

                [output]
                mode = "clipboard"
            "#,
        )
        .unwrap();

        let config = load_alone(&config_path).unwrap();
        assert_eq!(config.hotkey.key, "F12");
        assert_eq!(config.whisper.model, "tiny.en");
        assert_eq!(config.output.mode, OutputMode::Clipboard);
    }

    /// #646's user-facing contract: a config file with one broken section
    /// still loads. Before the salvage path, `load_config` returned Err here
    /// and the daemon refused to start, costing the user every setting they
    /// had over one bad value.
    #[test]
    fn load_config_warns_but_succeeds_on_a_partially_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
                [hotkey]
                key = "F13"

                [audio]
                max_duration_secs = "not a number"
            "#,
        )
        .unwrap();

        let config = load_alone(&config_path)
            .expect("a partially bad config must load, not refuse to start (#646)");
        assert_eq!(config.hotkey.key, "F13", "the good section must survive");
        assert_eq!(
            config.audio.max_duration_secs,
            Config::default().audio.max_duration_secs,
            "the bad section falls back to its default"
        );
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const BASE: &str = r#"
        engine = "parakeet"

        [hotkey]
        key = "F12"

        [osd]
        theme = "swiss"
        width_px = 600
    "#;

    #[test]
    fn without_config_d_the_config_is_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, BASE);
        let loaded = load_alone(&path).unwrap();
        let (parsed, _) = parse_config_salvaging(BASE).unwrap();
        assert_eq!(
            toml::Value::try_from(&loaded).unwrap(),
            toml::Value::try_from(&parsed).unwrap()
        );
    }

    #[test]
    fn an_overlay_changes_only_its_own_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, BASE);
        write(
            &dir.path().join("config.d/50-host.toml"),
            "[osd]\ntheme = \"showcase\"\n",
        );
        let config = load_alone(&path).unwrap();
        assert_eq!(config.osd.theme, crate::osd::config::OsdTheme::Showcase);
        assert_eq!(config.osd.width_px, 600);
        assert_eq!(config.hotkey.key, "F12");
        assert_eq!(config.engine, TranscriptionEngine::Parakeet);
    }

    #[test]
    fn overlays_apply_in_name_order_and_a_bad_one_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, BASE);
        let d = dir.path().join("config.d");
        write(&d.join("20-a.toml"), "[hotkey]\nkey = \"F13\"\n");
        write(&d.join("50-b.toml"), "[hotkey]\nkey = \"F14\"\n");
        write(&d.join("60-broken.toml"), "[hotkey\nkey = ");
        write(&d.join("70-notes.txt"), "[hotkey]\nkey = \"F15\"\n");
        let config = load_alone(&path).unwrap();
        assert_eq!(config.hotkey.key, "F14");
        assert_eq!(config.osd.theme, crate::osd::config::OsdTheme::Swiss);
    }
}
