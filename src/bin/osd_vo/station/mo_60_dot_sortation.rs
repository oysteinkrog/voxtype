//! MO-60. Dot sortation on the dotmatrix look: a new word's dots lift out of
//! the meter and fall into its letters; the meter under it is short and
//! regrows them.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-60", "Dot sortation", OsdTheme::Dotmatrix)
        .feature("dot-sort")
        .moment(Moment::Dictation)
}
