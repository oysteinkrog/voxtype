//! MO-33 Fold-under overflow on the captions look: when the lines roll up,
//! the top line turns away about its baseline instead of fading up.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-33", "Fold-under overflow (captions)", OsdTheme::Captions)
        .feature("fold-under")
        .moment(Moment::Overflow)
}
