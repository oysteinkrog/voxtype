//! MO-01 presentation lifecycle: a normal end lingers about 400 ms (longer
//! while a hand-off plays) and a new dictation cuts it short. Swiss shows it
//! plainly: the ending stays on screen instead of vanishing at once.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-01", "Presentation lifecycle", OsdTheme::Swiss)
        .moments(&[Moment::Dictation, Moment::Cancel])
}
