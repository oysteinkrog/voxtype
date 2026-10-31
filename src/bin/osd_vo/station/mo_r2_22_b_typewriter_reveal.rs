//! R2-22-b Typewriter reveal on the mission look: the newest word shows
//! in dark ink on a paper patch and rises 2 px, all over in 400 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-22-b", "Typewriter reveal: mission", OsdTheme::Mission)
        .feature("typewriter-reveal")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
