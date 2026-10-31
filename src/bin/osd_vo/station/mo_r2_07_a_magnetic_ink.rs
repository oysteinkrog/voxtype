//! R2-07-a Magnetic ink flow on the amber look: the waveform as a band of
//! fine particles, which gather behind each new word and drift apart.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-07-a", "Magnetic ink flow: amber", OsdTheme::Amber)
        .feature("magnetic-ink")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
