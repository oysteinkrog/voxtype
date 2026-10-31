//! R2-15-a Breathing membrane on the biolume look: three faint membranes
//! round the bell ripple and grow a little with the voice, then settle.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-15-a", "Breathing membrane: biolume", OsdTheme::Biolume)
        .feature("breathing-membrane")
        .moments(&[Moment::Dictation, Moment::Pause])
}
