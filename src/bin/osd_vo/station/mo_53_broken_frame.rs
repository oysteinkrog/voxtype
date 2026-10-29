//! MO-53 broken frame: on a failure the frame snaps at one point and the two
//! ends curl 4 px apart in 150 ms, with the failed stage named beside the
//! break. The words stay still.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-53", "Broken frame", OsdTheme::Type)
        .feature("broken-frame")
        .moment(Moment::Failure)
}
