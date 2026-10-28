//! MO-31 Margin bell on the type look: the reading line tightens up to 3.5 % as
//! it fills, relaxes on the wrap, and the text takes a 3 px carriage-return
//! shift.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-31", "Margin bell", OsdTheme::Type)
        .feature("margin-bell")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::Wrap)
}
