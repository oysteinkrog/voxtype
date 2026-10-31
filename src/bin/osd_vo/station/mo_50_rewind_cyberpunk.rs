//! MO-50 Rewind cancel on the cyberpunk look: the words wind back into the
//! yellow caret in place of the red strike.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-50", "Rewind cancel: cyberpunk", OsdTheme::Cyberpunk)
        .feature("rewind")
        .moment(Moment::Cancel)
}
