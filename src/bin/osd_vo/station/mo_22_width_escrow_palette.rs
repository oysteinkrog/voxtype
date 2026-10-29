//! MO-22 width escrow on the palette look.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-22", "Width escrow (palette)", OsdTheme::Palette)
        .feature("width-escrow")
        .moment(Moment::RevisionBurst)
}
