//! R2-20-c Breathing scroll on the hologram look: a translucent scroll behind the text
//! grows with each line and fades once the text is done.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-20-c", "Breathing scroll: hologram", OsdTheme::Hologram)
        .feature("breathing-scroll")
        .moments(&[Moment::Wrap, Moment::FinalPass])
}
