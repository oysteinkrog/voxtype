//! R2-04-a Liquid echo on the synthwave look: a faint wobbling mirror of
//! the newest line on a water line under it, and one flat ring from under
//! each new word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-04-a", "Liquid echo: synthwave", OsdTheme::Synthwave)
        .feature("liquid-echo")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
