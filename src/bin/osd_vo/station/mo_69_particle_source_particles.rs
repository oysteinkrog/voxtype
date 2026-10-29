//! MO-69 on the particles look: each onset of the voice kicks the swirl round the
//! button, which opens out, brightens and settles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-69", "Particles obey timing (particles)", OsdTheme::Particles)
        .feature(showcase::PARTICLE_SOURCE)
        .moments(&[Moment::Dictation, Moment::Pause])
}
