//! MO-84 light through letters on the glass look: a cool light that follows
//! the voice shines through the newest line onto the glass, with faint
//! caustics and short shadows, the cyan accent on the letters nearest it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-84", "Light through letters (glass)", OsdTheme::Glass)
        .feature(letter_fx::LIGHT)
        .moment(Moment::Dictation)
}
