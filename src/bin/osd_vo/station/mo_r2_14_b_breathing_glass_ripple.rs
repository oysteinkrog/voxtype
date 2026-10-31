//! R2-14-b Breathing glass ripple on the karaoke look: the waveform is a
//! band of particles that breathes with the voice, and each new word sends
//! rings across it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-14-b",
        "Breathing glass ripple: karaoke",
        OsdTheme::Karaoke,
    )
    .feature("breathing-glass-ripple")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
