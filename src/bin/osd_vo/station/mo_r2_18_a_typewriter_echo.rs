//! R2-18-a Typewriter echo on the scope look: a squashed, wobbling
//! reflection of the newest line under it that fades 1.5 s after each word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-18-a", "Typewriter echo: scope", OsdTheme::Scope)
        .feature("typewriter-echo")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
