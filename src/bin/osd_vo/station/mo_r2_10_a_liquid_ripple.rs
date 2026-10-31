//! R2-10-a Liquid ripple on the radial look: a membrane round the badge
//! where each word sends two bumps travelling round from the bottom.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-10-a", "Liquid ripple: radial", OsdTheme::Radial)
        .feature("liquid-ripple")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
