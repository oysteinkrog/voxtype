//! MO-64 bowstring underline on karaoke: the underline is a string from the
//! word before to the newest word; it tightens as the word is said and the
//! word's pitch bends it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-64", "Bowstring underline", OsdTheme::Karaoke)
        .feature(karaoke::BOWSTRING)
        .moments(&[Moment::Dictation, Moment::Pause])
}
