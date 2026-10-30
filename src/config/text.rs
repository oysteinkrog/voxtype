//! Text processing configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Text processing configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextConfig {
    /// Enable spoken punctuation conversion (e.g., "period" → ".")
    #[serde(default)]
    pub spoken_punctuation: bool,

    /// Custom word replacements (case-insensitive)
    /// Example: { "vox type" = "voxtype" }
    #[serde(default)]
    pub replacements: HashMap<String, String>,

    /// Extra replacements read from a separate TOML file of `"from" = "to"`
    /// lines, so a private word list can live outside the main config.
    /// Entries in `replacements` win over the file. `~/` is expanded.
    #[serde(default)]
    pub replacements_file: Option<String>,

    /// Learn words from spoken corrections: log each one and turn a pair
    /// seen twice (or a name, seen once) into a rule in
    /// `learned_words_file`. Defaults to true; corrections only happen with
    /// a correction key.
    #[serde(default = "default_true")]
    pub learn_corrections: bool,

    /// Where spoken corrections are logged, one JSON line each.
    /// Default: ~/.local/share/voxtype/corrections.jsonl. `~/` is expanded.
    #[serde(default)]
    pub corrections_log: Option<String>,

    /// Where learned words are kept, as `"from" = "to"` lines. `replacements`
    /// and `replacements_file` win over it.
    /// Default: ~/.local/share/voxtype/words-learned.toml. `~/` is expanded.
    #[serde(default)]
    pub learned_words_file: Option<String>,

    /// Smart auto-submit: say "submit" at the end of dictation to press Enter.
    /// The word "submit" is stripped from the output and Enter is pressed.
    #[serde(default)]
    pub smart_auto_submit: bool,

    /// Remove common filler words ("uh", "um", etc.) from transcribed text.
    /// Defaults to false to preserve existing behavior. The list is
    /// configurable via `filler_words`.
    #[serde(default)]
    pub filter_filler_words: bool,

    /// Words removed when `filter_filler_words` is true. Matched
    /// case-insensitively on word boundaries; surrounding punctuation and
    /// whitespace are cleaned up after removal.
    #[serde(default = "default_filler_words")]
    pub filler_words: Vec<String>,
}

impl Default for TextConfig {
    fn default() -> Self {
        Self {
            spoken_punctuation: false,
            replacements: HashMap::new(),
            replacements_file: None,
            learn_corrections: true,
            corrections_log: None,
            learned_words_file: None,
            smart_auto_submit: false,
            filter_filler_words: true,
            filler_words: default_filler_words(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Default filler-word list. Conservative: single-syllable disfluencies only.
/// Multi-word phrases like "you know" or "sort of" are too aggressive for a
/// default and can be added via the `filler_words` config.
pub(crate) fn default_filler_words() -> Vec<String> {
    vec![
        "uh".to_string(),
        "um".to_string(),
        "er".to_string(),
        "ah".to_string(),
        "eh".to_string(),
        "hmm".to_string(),
        "hm".to_string(),
        "mm".to_string(),
        "mhm".to_string(),
    ]
}
