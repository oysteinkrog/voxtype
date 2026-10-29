//! MO-50 rewind cancel: the words go back into the caret, newest first, at
//! 4x the spoken rate (at most 500 ms), and the scope trace scrubs back
//! with them.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-50", "Rewind cancel", OsdTheme::Scope)
        .feature("rewind")
        .moment(Moment::Cancel)
}
