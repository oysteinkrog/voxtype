//! MO-53 Broken frame on the phosphor look: on a failure the screen's green
//! border snaps open on its top edge, the ends lit pale, with the stage
//! that failed beside the break.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-53", "Broken frame: phosphor", OsdTheme::Phosphor)
        .feature("broken-frame")
        .moment(Moment::Failure)
}
