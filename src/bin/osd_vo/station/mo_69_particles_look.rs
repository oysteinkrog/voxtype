//! MO-69 in the particles look: words form out of particles under a swirl of
//! dotted light that follows the voice, the newest word in a blue to violet
//! gradient. Under reduced motion the swirl is still dots and words fade in.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-69", "Particles look", OsdTheme::Particles).moments(&[
        Moment::Dictation,
        Moment::Cancel,
        Moment::PastedEnter,
    ])
}
