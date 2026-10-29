//! MO-51 unthread cancel: a thread under the words pulls back from the
//! newest word to the first, letting go of a stitch at each word boundary.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-51", "Unthread cancel", OsdTheme::Type)
        .feature("unthread")
        .moment(Moment::Cancel)
}
