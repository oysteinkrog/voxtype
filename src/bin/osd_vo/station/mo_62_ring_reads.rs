//! MO-62 the ring reads with you, on jarvis: a light head on the ring looks
//! at the newest word like an eye, springs from word to word and sweeps the
//! text once with the final pass.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-62", "The ring reads with you", OsdTheme::Jarvis)
        .feature(jarvis::RING_READS)
        .moments(&[Moment::FinalPass, Moment::Pause])
}
