//! MO-23 revision address on the mission look: a change on an older line lights
//! a mark beside that line and one beside the newest line, and the two contract
//! together, in mission's orange.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-23", "Revision address (mission)", OsdTheme::Mission)
        .feature("revision-address")
        .moment(Moment::RevisionBurst)
}
