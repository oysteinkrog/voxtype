//! R2-21-b Liquid glass flow on the dot matrix look: the meter becomes a
//! glass ribbon that swells and narrows with the voice like a slow fluid.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-21-b",
        "Liquid glass flow: dot matrix",
        OsdTheme::Dotmatrix,
    )
    .feature("liquid-glass-flow")
    .moments(&[Moment::Dictation, Moment::Pause])
}
