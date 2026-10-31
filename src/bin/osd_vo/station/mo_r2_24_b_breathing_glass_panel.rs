//! R2-24-b Breathing glass panel on the river look: an inner rim that
//! breathes out towards the strip's edge with the voice, 3 px at most, and
//! rings that spread from the microphone and fade before the words.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-24-b", "Breathing glass panel: river", OsdTheme::River)
        .feature("breathing-glass-panel")
        .moments(&[Moment::Dictation, Moment::Pause])
}
