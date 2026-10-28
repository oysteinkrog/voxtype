//! MO-31 Margin bell on the phosphor look: the 3 px carriage-return shift on
//! each wrap.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-31", "Margin bell", OsdTheme::Phosphor)
        .feature("margin-bell")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::Wrap)
}
