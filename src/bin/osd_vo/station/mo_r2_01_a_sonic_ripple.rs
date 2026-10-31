//! R2-01-a Sonic ripple on the orb look: a thin ring grows from the orb's
//! rim across the glass behind the words for each new word and fades
//! within 1.2 s, brighter for a louder word; at most three at once.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-01-a", "Sonic ripple: orb", OsdTheme::Orb)
        .feature("sonic-ripple")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
