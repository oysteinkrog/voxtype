//! MO-10 breath line: before speech the reading line is a hairline that
//! breathes with the level; at the first word it kinks under the word and
//! the word's outlines rise out of it while the letters fill.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-10", "Breath line, first word finds its seat", OsdTheme::Type)
        .feature("breath-line")
        .moment(Moment::FirstWord)
}
