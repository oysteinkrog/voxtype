//! R2-07-b Typewriter ribbon on the amber look: a glowing head glides along
//! a band under the text to each new word and leaves a fading trail.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-07-b", "Typewriter ribbon: amber", OsdTheme::Amber)
        .feature("typewriter-ribbon")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
