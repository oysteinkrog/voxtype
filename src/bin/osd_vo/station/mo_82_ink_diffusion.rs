//! MO-82 ink diffusion on the type look: each new letter blooms like ink
//! on wet paper and settles into crisp type with a faint trace.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-82", "Ink diffusion", OsdTheme::Type)
        .feature(letter_fx::INK)
        .moment(Moment::Dictation)
}
