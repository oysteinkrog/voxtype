//! R2-15-c Ink reveal on the biolume look: each word feeds soft ink at its
//! box, which swirls out round and behind the text and drifts away.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-15-c", "Ink reveal: biolume", OsdTheme::Biolume)
        .feature("ink-reveal")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
