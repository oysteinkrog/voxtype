//! MO-86 depth stack history on the hologram look: older lines recede up
//! and back on sheets of blue glass in the column of light, while the
//! reading line lies flat at the bottom row.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-86",
        "Depth stack history (hologram)",
        OsdTheme::Hologram,
    )
    .feature(wild_space::DEPTH_STACK)
    .moments(&[Moment::Overflow, Moment::Wrap])
}
