//! MO-62 the ring reads with you, on radial: a light head inside the badge
//! ring looks toward the newest word, springs from word to word and sweeps
//! the text once with the final pass.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-62", "The ring reads with you: radial", OsdTheme::Radial)
        .feature(jarvis::RING_READS)
        .moments(&[Moment::FinalPass, Moment::Pause])
}
