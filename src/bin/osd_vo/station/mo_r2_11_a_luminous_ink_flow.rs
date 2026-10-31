//! R2-11-a Luminous ink flow on the cyberpunk look: smoke flows in from
//! the left behind the newest word, with a ghost of the word that
//! dissolves in 600 ms beside the sharp word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-11-a",
        "Luminous ink flow: cyberpunk",
        OsdTheme::Cyberpunk,
    )
    .feature("luminous-ink-flow")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
