//! R2-20-b Orbiting words on the hologram look: thin orbits round the text, paced by
//! the voice; a new word's faint copy runs along one and lands on the word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-20-b", "Orbiting words: hologram", OsdTheme::Hologram)
        .feature("orbiting-words")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
