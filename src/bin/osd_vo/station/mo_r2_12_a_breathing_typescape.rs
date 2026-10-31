//! R2-12-a Breathing typescape on the swissfi look: ribbon strands in the
//! wave band rise and fall with the voice and fade toward the words.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-12-a", "Breathing typescape: swissfi", OsdTheme::Swissfi)
        .feature("breathing-typescape")
        .moments(&[Moment::Dictation, Moment::Pause])
}
