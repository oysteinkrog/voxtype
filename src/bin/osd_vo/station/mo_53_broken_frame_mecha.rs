//! MO-53 Broken frame on the mecha look: on a failure the text panel's
//! border snaps at its top right corner, its ends lit yellow, with the stage
//! that failed above it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-53", "Broken frame: mecha", OsdTheme::Mecha)
        .feature("broken-frame")
        .moment(Moment::Failure)
}
