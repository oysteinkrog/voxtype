//! R2-21-c Orbital word rings on the dot matrix look: each word places a
//! dot with rings on the meter's line; the newest glows, older ones drift
//! left and fade.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-21-c",
        "Orbital word rings: dot matrix",
        OsdTheme::Dotmatrix,
    )
    .feature("orbital-word-rings")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
