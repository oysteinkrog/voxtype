//! MO-84 light through letters on the type look: a light that follows the
//! voice throws caustics and long soft shadows of the words on the panel.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-84", "Light through letters", OsdTheme::Type)
        .feature(letter_fx::LIGHT)
        .moment(Moment::Dictation)
}
