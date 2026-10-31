//! R2-16-a Ink ripple on the tron look: each word drops a bead of light
//! from the badge, and its ring spreads across the waveform's band.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-16-a", "Ink ripple: tron", OsdTheme::Tron)
        .feature("ink-ripple")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
