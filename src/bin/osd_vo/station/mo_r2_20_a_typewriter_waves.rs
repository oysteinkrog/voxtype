//! R2-20-a Typewriter waves on the hologram look: a calm pool under the newest line holds
//! its reflection, and each word sends a ring at its place.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-20-a", "Typewriter waves: hologram", OsdTheme::Hologram)
        .feature("typewriter-waves")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
