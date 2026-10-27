//! MO-74. Word bulge on the island look: each new word bulges the pill's
//! right edge 2 px and it relaxes.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-74", "Word bulge", OsdTheme::Island)
        .feature("word-bulge")
        .moment(Moment::Dictation)
}
