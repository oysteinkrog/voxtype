//! MO-81 Variable font breathing on the showcase look: each word in its own
//! Roboto Flex instance, weight from loudness, width from tempo and slant
//! from pitch, while the words still form from the ring's particles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable font breathing", OsdTheme::Showcase)
        .feature(wild_motion::VARIABLE_BREATH)
        .moments(&[Moment::Dictation, Moment::Pause])
}
