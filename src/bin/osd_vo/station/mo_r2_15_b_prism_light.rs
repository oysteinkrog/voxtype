//! R2-15-b Prism light on the biolume look: soft rays fall through the bell
//! and the tendrils with the voice, a rainbow band and glints run along the
//! rim, and the light fades out over 600 ms when the dictation finishes.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-15-b", "Prism light: biolume", OsdTheme::Biolume)
        .feature("prism-light")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
