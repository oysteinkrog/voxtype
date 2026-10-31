//! R2-01-b Magnetic type on the orb look: tiny copies of a new word's
//! letters stream from the orb to the word and fade as they arrive, within
//! 400 ms; the word itself shows at 70 % ink from its first frame.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-01-b", "Magnetic type: orb", OsdTheme::Orb)
        .feature("magnetic-type")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
