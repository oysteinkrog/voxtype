//! MO-33 Fold-under overflow on the amber look: the top line turns away about
//! its baseline while the lines below move up.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-33", "Fold-under overflow (amber)", OsdTheme::Amber)
        .feature("fold-under")
        .moment(Moment::Overflow)
}
