//! MO-51 Unthread cancel on the captions look: a thread under the centred
//! lines pulls back from the newest word to the first, letting go of a
//! stitch at each word boundary.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-51", "Unthread cancel (captions)", OsdTheme::Captions)
        .feature("unthread")
        .moment(Moment::Cancel)
}
