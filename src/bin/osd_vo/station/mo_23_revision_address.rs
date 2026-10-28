//! MO-23 revision address: a change on an older line lights a mark beside that line and
//! one beside the newest line, and the two contract together.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-23", "Revision address", OsdTheme::Swissfi)
        .feature("revision-address")
        .moment(Moment::RevisionBurst)
}
