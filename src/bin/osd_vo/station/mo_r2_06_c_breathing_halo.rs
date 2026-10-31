//! R2-06-c Breathing halo on the phosphor look: an arc of light rises
//! behind the top of the text while you speak and settles in a pause.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-06-c", "Breathing halo: phosphor", OsdTheme::Phosphor)
        .feature("breathing-halo")
        .moments(&[Moment::Dictation, Moment::Pause])
}
