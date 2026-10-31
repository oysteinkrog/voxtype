//! R2-13-c Typewriter set on the captions look: a carriage marker on a
//! ticking ruler follows the newest word, and a pause frames the phrase.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-13-c", "Typewriter set: captions", OsdTheme::Captions)
        .feature("typewriter-set")
        .moments(&[Moment::FirstWord, Moment::Dictation, Moment::Pause])
}
