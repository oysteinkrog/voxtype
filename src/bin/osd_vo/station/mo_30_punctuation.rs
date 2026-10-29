//! MO-30 Silence becomes punctuation on the swiss look: pause ticks on the
//! baseline gather into a hollow comma.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-30", "Silence becomes punctuation", OsdTheme::Swiss)
        .feature("pause-marks")
        .feature("no-pre-echo")
        .feature("no-exhale")
        .moment(Moment::Pause)
}
