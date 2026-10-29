//! MO-86 depth stack history on the showcase look: older lines recede up
//! and back on matte glass sheets in the strip, while the reading line
//! lies flat where the flat text puts its last line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-86", "Depth stack history", OsdTheme::Showcase)
        .feature(wild_space::DEPTH_STACK)
        .moments(&[Moment::Overflow, Moment::Wrap])
}
