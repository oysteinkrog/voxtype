//! MO-40 The final pass sets the type, on swissfi.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-40", "The final pass sets the type", OsdTheme::Swissfi)
        .feature("no-spoken-stagger")
        .feature("no-count-cast")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::FinalPass)
}
