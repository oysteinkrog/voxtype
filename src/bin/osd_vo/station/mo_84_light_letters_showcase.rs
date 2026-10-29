//! MO-84 light through letters on the showcase look: a light that follows
//! the voice shines through the reading line onto the matte strip, with
//! caustics and long soft shadows, the accent on the letters nearest it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-84", "Light through letters", OsdTheme::Showcase)
        .feature(letter_fx::LIGHT)
        .moment(Moment::Dictation)
}
