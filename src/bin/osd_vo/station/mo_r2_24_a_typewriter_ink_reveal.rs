//! R2-24-a Typewriter ink reveal on the river look: the newest word is at
//! 70 % ink at once and the pen writes it to full ink in stroke order, over
//! a soft ink bleed under the word that fades as it goes.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-24-a", "Typewriter ink reveal: river", OsdTheme::River)
        .feature("typewriter-ink-reveal")
        // The pen that does the writing.
        .feature("stroke-write")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
