//! MO-53 Broken frame on the cyberpunk look: on a failure the top edge snaps
//! open right of the tag, its ends lit yellow, with the stage that failed.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-53", "Broken frame: cyberpunk", OsdTheme::Cyberpunk)
        .feature("broken-frame")
        .moment(Moment::Failure)
}
