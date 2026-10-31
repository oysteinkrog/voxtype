//! R2-11-b Breathing focus field on the cyberpunk look: faint contour
//! lines behind the text drift outward with the voice, and the field's
//! corner brackets breathe in by up to 4 px.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-11-b",
        "Breathing focus field: cyberpunk",
        OsdTheme::Cyberpunk,
    )
    .feature("breathing-focus-field")
    .moments(&[Moment::Dictation, Moment::Pause])
}
