//! R2-11-a Luminous ink flow on the cyberpunk look: smoke flows in from
//! the left behind the newest word and is gone in 600 ms.

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
