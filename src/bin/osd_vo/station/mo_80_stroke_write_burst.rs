//! MO-80 stroke writing when words come four at a time: one pen catches up
//! word by word, faster the further behind it is, and sweeps in a word it
//! cannot start within 250 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: burst", OsdTheme::Type)
        .feature("stroke-write")
        .moment(Moment::WordBurst)
}
