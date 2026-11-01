//! R2-23-c Breathing grid on the mecha look: contour lines behind the text
//! ripple with the voice and lift near each new word, and lie flat in a
//! pause.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-23-c", "Breathing grid: mecha", OsdTheme::Mecha)
        .feature("breathing-grid")
        .moments(&[Moment::Dictation, Moment::Pause])
}
