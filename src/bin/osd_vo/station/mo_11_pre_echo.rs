//! MO-11 Pre-echo caret on the type look: faint blobs per onset on the baseline
//! before the words come, each turning into its word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-11", "Pre-echo caret", OsdTheme::Type)
        .feature("pre-echo")
        .feature("no-exhale")
        .feature("no-pause-marks")
        .moment(Moment::FirstWord)
}
