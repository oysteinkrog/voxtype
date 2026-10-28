//! MO-86 depth stack history on the jarvis look: older lines recede on
//! glass sheets behind the flat reading line, with parallax as each new
//! line pushes the stack back.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-86", "Depth stack history", OsdTheme::Jarvis)
        .feature(wild_space::DEPTH_STACK)
        .moments(&[Moment::Overflow, Moment::Wrap])
}
