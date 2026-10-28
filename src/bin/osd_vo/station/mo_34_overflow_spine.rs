//! MO-34 Overflow spine on the type look: each line that leaves the top leaves word-wide
//! blocks along the text's top edge, which close into one history tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-34", "Overflow spine", OsdTheme::Type)
        .feature("overflow-spine")
        .moment(Moment::Overflow)
}
