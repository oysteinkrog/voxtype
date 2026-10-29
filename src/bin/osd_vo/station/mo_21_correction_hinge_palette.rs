//! MO-21 correction hinge on the palette look.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-21", "Correction hinge (palette)", OsdTheme::Palette)
        .feature("correction-hinge")
        .moment(Moment::RevisionBurst)
}
