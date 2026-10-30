//! `voxtype glossary` subcommand actions.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum GlossaryAction {
    /// List the words learned from spoken corrections
    ///
    /// A spoken correction (hold the correction key while recording and say
    /// the right words) is logged to text.corrections_log. The same fix
    /// twice, or a name once, becomes a rule in text.learned_words_file.
    /// text.replacements and text.replacements_file win over it.
    Learned,

    /// Remove a learned word
    ///
    /// Example: voxtype glossary forget glim bot
    Forget {
        /// The wrong words of the rule, as `voxtype glossary learned` lists them
        #[arg(required = true, num_args = 1..)]
        words: Vec<String>,
    },
}
