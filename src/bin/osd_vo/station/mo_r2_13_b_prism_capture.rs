//! R2-13-b Prism capture on the captions look: a beam meets a glass slab
//! above the text and fans into a fine spectrum that follows the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-13-b", "Prism capture: captions", OsdTheme::Captions)
        .feature("prism-capture")
        .moments(&[Moment::Dictation, Moment::Pause])
}
