//! R2-10-c Breathing halo on the radial look: petal rings breathe with the
//! voice behind the bars, and motes drift out when a phrase ends.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-10-c", "Breathing halo: radial", OsdTheme::Radial)
        .feature("breathing-halo")
        .moments(&[Moment::Dictation, Moment::Pause])
}
