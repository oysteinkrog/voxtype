//! MO-65. The count takes a cast in the dotmatrix look's own material: the
//! block is made of dots and lands in a notch of dots by the count.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-65", "The count takes a cast (dots)", OsdTheme::Dotmatrix)
        .feature("count-cast")
        .moment(Moment::Dictation)
}
