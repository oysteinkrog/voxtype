//! R2-14-c Connecting thoughts on the karaoke look: a dot under each word,
//! joined along its line, easing into one strand when the text lands.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-14-c", "Connecting thoughts: karaoke", OsdTheme::Karaoke)
        .feature("connecting-thoughts")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
