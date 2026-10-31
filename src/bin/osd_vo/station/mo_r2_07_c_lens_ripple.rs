//! R2-07-c Glass lens ripple on the amber look: rings grow and fade behind
//! each new word, brighter with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-07-c", "Glass lens ripple: amber", OsdTheme::Amber)
        .feature("lens-ripple")
        .moments(&[Moment::FirstWord, Moment::Pause])
}
