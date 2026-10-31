//! R2-08-a Breathing aurora on the glass look: soft aurora strands behind
//! the text swell with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-08-a", "Breathing aurora: glass", OsdTheme::Glass)
        .feature("breathing-aurora")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
