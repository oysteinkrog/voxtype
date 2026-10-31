//! MO-86 depth stack history on the glass look: older lines recede on
//! sheets above the newest line, which stays flat on the glass card.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-86", "Depth stack history (glass)", OsdTheme::Glass)
        .feature(wild_space::DEPTH_STACK)
        .moments(&[Moment::Overflow, Moment::Wrap])
}
