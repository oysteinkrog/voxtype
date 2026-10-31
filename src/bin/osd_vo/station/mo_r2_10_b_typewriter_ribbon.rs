//! R2-10-b Typewriter ribbon on the radial look: a woven ribbon arc steps
//! round per word, its head bar strikes like a typebar, and the newest
//! word inks from 70 % to full.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-10-b", "Typewriter ribbon: radial", OsdTheme::Radial)
        .feature("typewriter-ribbon")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
