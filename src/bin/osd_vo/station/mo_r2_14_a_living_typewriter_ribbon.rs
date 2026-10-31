//! R2-14-a Living typewriter ribbon on the karaoke look: a glowing head
//! glides along a band under the text to each new word and inks it there.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-14-a",
        "Living typewriter ribbon: karaoke",
        OsdTheme::Karaoke,
    )
    .feature("living-typewriter-ribbon")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
