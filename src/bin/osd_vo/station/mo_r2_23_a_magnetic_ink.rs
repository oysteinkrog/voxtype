//! R2-23-a Magnetic ink on the mecha look: a few orange dots curl in from
//! the left of the panel and gather into each new word, then fade.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-23-a", "Magnetic ink: mecha", OsdTheme::Mecha)
        .feature("magnetic-ink")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
