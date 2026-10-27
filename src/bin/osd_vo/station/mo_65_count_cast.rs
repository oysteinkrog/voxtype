//! MO-65. The count takes a cast, on the swissfi look: a block the width of
//! the new word contracts into a notch by the word count as its digit rolls.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-65", "The count takes a cast", OsdTheme::Swissfi)
        .feature("count-cast")
        .moment(Moment::Dictation)
}
