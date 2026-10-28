//! MO-80 stroke-order writing: each new word is written by pen strokes in
//! the order a hand draws them, over the word, which arrives readable and
//! fills as the pen passes; then the strokes leave the clean typeface.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke-order writing", OsdTheme::Type)
        .feature("stroke-write")
        .moment(Moment::Dictation)
}
