//! MO-31 Margin bell on the amber look: the newest line tightens a little as
//! it nears the right edge and relaxes on the wrap.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-31", "Margin bell (amber)", OsdTheme::Amber)
        .feature("margin-bell")
        .moment(Moment::Wrap)
}
