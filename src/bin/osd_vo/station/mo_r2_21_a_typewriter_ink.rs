//! R2-21-a Typewriter ink on the dot matrix look: each new word strikes
//! the newest columns with an ink bloom that travels left with its audio
//! and fades over 1.5 s into faint grain.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-21-a", "Typewriter ink: dot matrix", OsdTheme::Dotmatrix)
        .feature("typewriter-ink")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
