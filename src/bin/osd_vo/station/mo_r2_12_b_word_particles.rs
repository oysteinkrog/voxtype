//! R2-12-b Word particles on the swissfi look: fine dust gathers into
//! each new word behind it and is gone in 400 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-12-b", "Word particles: swissfi", OsdTheme::Swissfi)
        .feature("word-particles")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
