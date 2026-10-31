//! R2-12-c Echo focus on the swissfi look: one faint ring grows from the
//! end of the text at the stop and fades in under a second.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-12-c", "Echo focus: swissfi", OsdTheme::Swissfi)
        .feature("echo-focus")
        .moments(&[Moment::FinalPass])
}
