//! MO-22 width escrow: in a burst of revisions the changed word borrows up to half an em,
//! and its neighbours make one move once the burst is quiet.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-22", "Width escrow", OsdTheme::Type)
        .feature("width-escrow")
        .moment(Moment::RevisionBurst)
}
