//! R2-18-b Breathing text on the scope look: a soft pill behind the newest
//! word and a ring round the trace brighten and grow with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-18-b", "Breathing text: scope", OsdTheme::Scope)
        .feature("breathing-text")
        .moments(&[Moment::Dictation, Moment::Pause])
}
