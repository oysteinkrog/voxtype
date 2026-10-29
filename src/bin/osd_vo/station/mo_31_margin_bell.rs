//! MO-31 Margin bell on the type look: the newest line tightens a little as it nears the
//! right edge, relaxes on the wrap, and the new line arrives with a 3 px carriage return.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-31", "Margin bell", OsdTheme::Type)
        .feature("margin-bell")
        .moment(Moment::Wrap)
}
