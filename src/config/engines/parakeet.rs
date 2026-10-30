//! Parakeet engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Parakeet model architecture type
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ParakeetModelType {
    /// CTC (Connectionist Temporal Classification) - faster, character-level output
    Ctc,
    /// TDT (Token-Duration-Transducer) - recommended, proper punctuation and word boundaries
    #[default]
    Tdt,
}

/// Parakeet speech-to-text configuration (ONNX-based, alternative to Whisper)
/// Requires: cargo build --features parakeet
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParakeetConfig {
    /// Path to model directory containing ONNX model files
    /// For TDT: encoder-model.onnx, decoder_joint-model.onnx, vocab.txt
    /// For CTC: model.onnx, tokenizer.json
    pub model: String,

    /// Model architecture type: "tdt" (default, recommended) or "ctc"
    /// Auto-detected from model directory structure if not specified
    #[serde(default)]
    pub model_type: Option<ParakeetModelType>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,

    /// Use the cache-aware streaming pipeline (parakeet-rs `ParakeetUnified`)
    /// instead of the batch CTC/TDT models. When true, voxtype emits live
    /// partial transcripts during recording and types the final transcript
    /// on hotkey release. Requires a streaming-capable model directory
    /// (TDT v3 family with `tokenizer.model`).
    /// Default: false (batch pipeline, identical to pre-streaming behavior).
    #[serde(default)]
    pub streaming: bool,

    /// Streaming chunk length in seconds: how often the cache-aware model
    /// commits new words. Smaller = lower latency, more inference overhead.
    /// Rounded to whole 0.08 s encoder steps. Absent: parakeet-rs's default
    /// (0.56 s). Used by `streaming = true` and by `live_preview_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_chunk_secs: Option<f32>,

    /// Streaming left context in seconds: past audio each chunk sees.
    /// Rounded to whole 0.08 s steps. Absent: parakeet-rs's default (5.6 s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_left_context_secs: Option<f32>,

    /// Streaming right context in seconds: lookahead each chunk waits for.
    /// Rounded to whole 0.08 s steps. Absent: parakeet-rs's default (0.56 s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_right_context_secs: Option<f32>,

    /// How often the live preview's lookahead tail (the dim guess after the
    /// committed words) is decoded again between two streaming steps, in
    /// seconds of audio. Each tick is one extra encoder run; committed text
    /// does not change. 0 turns the ticks off: the tail then changes once a
    /// step. Absent: [`DEFAULT_STREAMING_TAIL_SECS`]. A tick that takes
    /// more than 60 % of its interval twice in a row stops the ticks for
    /// that recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_tail_secs: Option<f32>,

    /// Seconds of quiet after which the live preview shows the lookahead
    /// tail's words as text, instead of waiting a step for their right
    /// context. Speeds up the last words before a pause. The committed text
    /// and the final text do not change; a later step that reads the words
    /// differently shows as a revision. 0 turns it off. Absent:
    /// [`DEFAULT_STREAMING_PAUSE_COMMIT_SECS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming_pause_commit_secs: Option<f32>,
}

/// `streaming_tail_secs` when the key is absent.
pub const DEFAULT_STREAMING_TAIL_SECS: f32 = 0.24;

/// `streaming_pause_commit_secs` when the key is absent.
pub const DEFAULT_STREAMING_PAUSE_COMMIT_SECS: f32 = 0.3;

/// Seconds per encoder output frame of the cache-aware Parakeet models:
/// 8 mel frames of 10 ms. Every streaming window size must be a whole
/// number of these.
pub const PARAKEET_STREAMING_STEP_SECS: f32 = 0.08;

/// The longest window any one streaming key may ask for.
const MAX_STREAMING_SECS: f32 = 60.0;

/// The cache-aware streaming window, in whole encoder steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingWindow {
    pub left_steps: u32,
    pub chunk_steps: u32,
    pub right_steps: u32,
}

/// parakeet-rs `UnifiedStreamingConfig::default()`, in encoder steps:
/// 5.6 s left, 0.56 s chunk, 0.56 s right.
impl Default for StreamingWindow {
    fn default() -> Self {
        Self {
            left_steps: 70,
            chunk_steps: 7,
            right_steps: 7,
        }
    }
}

impl StreamingWindow {
    pub fn left_secs(&self) -> f32 {
        self.left_steps as f32 * PARAKEET_STREAMING_STEP_SECS
    }

    pub fn chunk_secs(&self) -> f32 {
        self.chunk_steps as f32 * PARAKEET_STREAMING_STEP_SECS
    }

    pub fn right_secs(&self) -> f32 {
        self.right_steps as f32 * PARAKEET_STREAMING_STEP_SECS
    }
}

impl std::fmt::Display for StreamingWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.2} s left + {:.2} s chunk + {:.2} s right (step {:.2} s)",
            self.left_secs(),
            self.chunk_secs(),
            self.right_secs(),
            self.chunk_secs()
        )
    }
}

/// Round one `streaming_*` value to whole encoder steps.
fn streaming_steps(key: &str, secs: f32) -> Result<u32, String> {
    if !secs.is_finite() || !(0.0..=MAX_STREAMING_SECS).contains(&secs) {
        return Err(format!(
            "[parakeet] {key} = {secs} is out of range: use 0 to {MAX_STREAMING_SECS} seconds, \
             in steps of {PARAKEET_STREAMING_STEP_SECS} s (for example 0.32)"
        ));
    }
    Ok((secs / PARAKEET_STREAMING_STEP_SECS).round() as u32)
}

impl ParakeetConfig {
    /// The streaming window from the `streaming_*` keys, each rounded to the
    /// nearest whole 0.08 s step. A missing key keeps parakeet-rs's default
    /// for that key. Errors on a value out of range, or a chunk that rounds
    /// to zero.
    pub fn streaming_window(&self) -> Result<StreamingWindow, String> {
        let d = StreamingWindow::default();
        let pick = |key: &str, value: Option<f32>, default: u32| match value {
            Some(secs) => streaming_steps(key, secs),
            None => Ok(default),
        };
        let window = StreamingWindow {
            left_steps: pick(
                "streaming_left_context_secs",
                self.streaming_left_context_secs,
                d.left_steps,
            )?,
            chunk_steps: pick(
                "streaming_chunk_secs",
                self.streaming_chunk_secs,
                d.chunk_steps,
            )?,
            right_steps: pick(
                "streaming_right_context_secs",
                self.streaming_right_context_secs,
                d.right_steps,
            )?,
        };
        if window.chunk_steps == 0 {
            return Err(format!(
                "[parakeet] streaming_chunk_secs must be at least {PARAKEET_STREAMING_STEP_SECS} s \
                 (one encoder step); it rounds to 0"
            ));
        }
        Ok(window)
    }

    /// `streaming_tail_secs`, or its default; 0 for off. Out of range (not
    /// 0 to 60 s) is off, with a note in the worker's log.
    pub fn streaming_tail_secs(&self) -> f32 {
        let secs = self.streaming_tail_secs.unwrap_or(DEFAULT_STREAMING_TAIL_SECS);
        if secs.is_finite() && (0.0..=MAX_STREAMING_SECS).contains(&secs) {
            secs
        } else {
            0.0
        }
    }

    /// `streaming_pause_commit_secs`, or its default; 0 for off. Out of range
    /// (not 0 to 60 s) is off.
    pub fn streaming_pause_commit_secs(&self) -> f32 {
        let secs = self
            .streaming_pause_commit_secs
            .unwrap_or(DEFAULT_STREAMING_PAUSE_COMMIT_SECS);
        if secs.is_finite() && (0.0..=MAX_STREAMING_SECS).contains(&secs) {
            secs
        } else {
            0.0
        }
    }

    /// The `streaming_*` keys whose value is not a whole number of steps, with
    /// the value it was rounded to, for a warning in the log.
    pub fn streaming_rounding_notes(&self) -> Vec<String> {
        [
            (
                "streaming_left_context_secs",
                self.streaming_left_context_secs,
            ),
            ("streaming_chunk_secs", self.streaming_chunk_secs),
            (
                "streaming_right_context_secs",
                self.streaming_right_context_secs,
            ),
        ]
        .into_iter()
        .filter_map(|(key, value)| {
            let secs = value?;
            let steps = streaming_steps(key, secs).ok()?;
            let rounded = steps as f32 * PARAKEET_STREAMING_STEP_SECS;
            ((rounded - secs).abs() > 1e-4)
                .then(|| format!("{key} = {secs} rounded to {rounded:.2}"))
        })
        .collect()
    }
}

impl Default for ParakeetConfig {
    fn default() -> Self {
        Self {
            model: "parakeet-tdt-0.6b-v3".to_string(),
            model_type: None, // Auto-detect
            on_demand_loading: false,
            streaming: false,
            streaming_chunk_secs: None,
            streaming_left_context_secs: None,
            streaming_right_context_secs: None,
            streaming_tail_secs: None,
            streaming_pause_commit_secs: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_parse_parakeet_model_type_tdt() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
            model_type = "tdt"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-tdt-0.6b-v3");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Tdt));
    }

    #[test]
    fn test_parse_parakeet_model_type_ctc() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-ctc-0.6b"
            model_type = "ctc"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        assert_eq!(parakeet.model, "parakeet-ctc-0.6b");
        assert_eq!(parakeet.model_type, Some(ParakeetModelType::Ctc));
    }

    #[test]
    fn test_parakeet_model_type_defaults_to_none_for_auto_detection() {
        let toml_str = r#"
            engine = "parakeet"

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

            [parakeet]
            model = "parakeet-tdt-0.6b-v3"
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        let parakeet = config.parakeet.unwrap();
        // model_type should be None (will be auto-detected at runtime)
        assert!(parakeet.model_type.is_none());
    }

    #[test]
    fn test_parakeet_config_default() {
        let config = ParakeetConfig::default();
        assert_eq!(config.model, "parakeet-tdt-0.6b-v3");
        assert!(config.model_type.is_none());
        assert!(!config.on_demand_loading);
    }

    fn parakeet_section(keys: &str) -> ParakeetConfig {
        toml::from_str(&format!(
            "model = \"parakeet-unified-en-0.6b-int8\"\n{keys}"
        ))
        .unwrap()
    }

    #[test]
    fn streaming_tail_secs_defaults_reads_and_turns_off() {
        assert_eq!(
            parakeet_section("").streaming_tail_secs(),
            DEFAULT_STREAMING_TAIL_SECS
        );
        assert_eq!(
            parakeet_section("streaming_tail_secs = 0.16").streaming_tail_secs(),
            0.16
        );
        assert_eq!(
            parakeet_section("streaming_tail_secs = 0").streaming_tail_secs(),
            0.0
        );
        // Out of range is off rather than an error: the ticks are optional.
        assert_eq!(
            parakeet_section("streaming_tail_secs = -1").streaming_tail_secs(),
            0.0
        );
    }

    #[test]
    fn streaming_pause_commit_secs_defaults_reads_and_turns_off() {
        let secs = |toml: &str| parakeet_section(toml).streaming_pause_commit_secs();
        assert_eq!(secs(""), DEFAULT_STREAMING_PAUSE_COMMIT_SECS);
        assert_eq!(secs("streaming_pause_commit_secs = 0.5"), 0.5);
        assert_eq!(secs("streaming_pause_commit_secs = 0"), 0.0);
        assert_eq!(secs("streaming_pause_commit_secs = -1"), 0.0);
    }

    #[test]
    fn streaming_window_absent_keys_keep_parakeet_rs_default() {
        let window = parakeet_section("").streaming_window().unwrap();
        assert_eq!(
            (window.left_steps, window.chunk_steps, window.right_steps),
            (70, 7, 7)
        );
        assert!((window.left_secs() - 5.6).abs() < 1e-4);
        assert!((window.chunk_secs() - 0.56).abs() < 1e-4);
        assert!((window.right_secs() - 0.56).abs() < 1e-4);
    }

    #[test]
    fn streaming_window_reads_all_three_keys() {
        let cfg = parakeet_section(
            "streaming_left_context_secs = 2.4\n\
             streaming_chunk_secs = 0.32\n\
             streaming_right_context_secs = 0.32",
        );
        let window = cfg.streaming_window().unwrap();
        assert_eq!(
            (window.left_steps, window.chunk_steps, window.right_steps),
            (30, 4, 4)
        );
        assert!(cfg.streaming_rounding_notes().is_empty());
        assert_eq!(
            window.to_string(),
            "2.40 s left + 0.32 s chunk + 0.32 s right (step 0.32 s)"
        );
    }

    #[test]
    fn streaming_window_one_key_keeps_the_other_defaults() {
        let window = parakeet_section("streaming_chunk_secs = 0.32")
            .streaming_window()
            .unwrap();
        assert_eq!(
            (window.left_steps, window.chunk_steps, window.right_steps),
            (70, 4, 7)
        );
    }

    #[test]
    fn streaming_window_rounds_to_whole_steps() {
        // The old built-in defaults, which parakeet-rs rejected outright.
        let cfg = parakeet_section(
            "streaming_left_context_secs = 1.5\n\
             streaming_chunk_secs = 0.5\n\
             streaming_right_context_secs = 0.5",
        );
        let window = cfg.streaming_window().unwrap();
        assert_eq!(
            (window.left_steps, window.chunk_steps, window.right_steps),
            (19, 6, 6)
        );
        let notes = cfg.streaming_rounding_notes();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[1], "streaming_chunk_secs = 0.5 rounded to 0.48");
    }

    #[test]
    fn streaming_window_accepts_zero_contexts() {
        let window =
            parakeet_section("streaming_left_context_secs = 0.0\nstreaming_right_context_secs = 0")
                .streaming_window()
                .unwrap();
        assert_eq!((window.left_steps, window.right_steps), (0, 0));
    }

    #[test]
    fn streaming_window_rejects_a_chunk_that_rounds_to_zero() {
        let err = parakeet_section("streaming_chunk_secs = 0.03")
            .streaming_window()
            .unwrap_err();
        assert!(err.contains("streaming_chunk_secs"), "{err}");
        assert!(err.contains("rounds to 0"), "{err}");
    }

    #[test]
    fn streaming_window_rejects_out_of_range_values() {
        for keys in [
            "streaming_left_context_secs = -0.08",
            "streaming_right_context_secs = 61.0",
            "streaming_chunk_secs = nan",
        ] {
            let err = parakeet_section(keys).streaming_window().unwrap_err();
            assert!(err.contains("out of range"), "{keys}: {err}");
        }
    }

    #[test]
    fn streaming_keys_absent_are_not_serialized() {
        let out = toml::to_string(&ParakeetConfig::default()).unwrap();
        assert!(!out.contains("streaming_chunk_secs"), "{out}");
        let cfg = parakeet_section("streaming_chunk_secs = 0.32");
        let back: ParakeetConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.streaming_chunk_secs, Some(0.32));
        assert_eq!(back.streaming_left_context_secs, None);
    }

    #[test]
    fn test_parakeet_model_type_enum_default() {
        // ParakeetModelType defaults to Tdt
        assert_eq!(ParakeetModelType::default(), ParakeetModelType::Tdt);
    }
}
