//! R2-13-a Liquid echo on the captions look: each new word sends a flat
//! ring across a faint water surface above the text.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-13-a", "Liquid echo: captions", OsdTheme::Captions)
        .feature("liquid-echo")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
