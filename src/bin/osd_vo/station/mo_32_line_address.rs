//! MO-32 The line changes its address, on the type look: a mark beside the newest line
//! splits on a wrap, one half staying with the previous line, one going to the new line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-32", "The line changes its address", OsdTheme::Type)
        .feature("margin-mark")
        .moment(Moment::Wrap)
}
