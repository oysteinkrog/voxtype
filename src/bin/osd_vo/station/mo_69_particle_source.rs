//! MO-69 particles obey timing, negative impression, on the showcase look:
//! a light runs along the ring and each word's particles come from where it
//! was when the word was said; a landed word leaves a few dots of its
//! negative under it that fold into a tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-69",
        "Particles obey timing, negative impression",
        OsdTheme::Showcase,
    )
    .feature(showcase::PARTICLE_SOURCE)
    .moments(&[Moment::Dictation, Moment::Pause])
}
