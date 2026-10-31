//! R2-02-b Particle orbit on the jarvis look: fine particles orbit the mic,
//! pulled into a tighter, brighter ring as you speak and loosening on a
//! pause. At most 40 % alpha, outside the text.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-02-b", "Particle orbit: jarvis", OsdTheme::Jarvis)
        .feature("particle-orbit")
        .moments(&[Moment::Dictation, Moment::Pause])
}
