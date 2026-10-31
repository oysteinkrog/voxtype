//! R2-09-c Letterpress build on the particles look: the newest word's
//! letters are set in 20 ms apart, falling 3 px at most and landing with one
//! short press, and the dots round the text step on with each word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-09-c",
        "Letterpress build: particles",
        OsdTheme::Particles,
    )
    .feature("letterpress-build")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
