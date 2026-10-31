//! MO-80 stroke writing on the tron look: a light pen writes each new word
//! along its letters, a cyan nib light trailing 40 ms behind it and a glow
//! that settles into the type.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: tron", OsdTheme::Tron)
        .feature("stroke-write")
        .moment(Moment::Dictation)
}
