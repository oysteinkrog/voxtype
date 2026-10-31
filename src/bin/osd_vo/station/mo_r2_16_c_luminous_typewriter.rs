//! R2-16-c Luminous typewriter on the tron look: the last words in a
//! column right of the text, the newest in a bright slot, stepping up one
//! slot per word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-16-c", "Luminous typewriter: tron", OsdTheme::Tron)
        .feature("luminous-typewriter")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
