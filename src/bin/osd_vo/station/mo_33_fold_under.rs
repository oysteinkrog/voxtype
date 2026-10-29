//! MO-33 Fold-under overflow on the type look: when the text passes its line limit,
//! the top line turns away about its baseline while the lines below move up.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-33", "Fold-under overflow", OsdTheme::Type)
        .feature("fold-under")
        .moment(Moment::Overflow)
}
