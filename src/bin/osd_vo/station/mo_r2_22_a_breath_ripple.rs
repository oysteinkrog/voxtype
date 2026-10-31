//! R2-22-a Breath ripple on the mission look: rings of light swirl out
//! behind the transcript, reaching further and turning faster as the voice
//! gets louder.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-22-a", "Breath ripple: mission", OsdTheme::Mission)
        .feature("breath-ripple")
        .moments(&[Moment::Dictation, Moment::Pause])
}
