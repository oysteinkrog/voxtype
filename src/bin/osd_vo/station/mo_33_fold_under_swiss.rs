//! MO-33 Fold-under overflow, the flat form on the swiss look: the top line thins to
//! nothing toward its baseline while the lines below move up.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-33", "Fold-under overflow (flat)", OsdTheme::Swiss)
        .feature("fold-under")
        .moment(Moment::Overflow)
}
