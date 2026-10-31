//! MO-84 light through letters on the aurora look: a warm light that follows
//! the voice shines through the newest line onto the mesh, cyan where it is
//! brightest, with short soft shadows.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-84", "Light through letters (aurora)", OsdTheme::Aurora)
        .feature(letter_fx::LIGHT)
        .moment(Moment::Dictation)
}
