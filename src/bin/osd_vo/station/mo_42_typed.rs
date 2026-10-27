//! MO-42 hand-off, typed: letters leave in reading order at the typing rate
//! the daemon reported, each shrinking to its baseline.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-42", "Hand-off: the text leaves (typed)", OsdTheme::Swissfi)
        .moment(Moment::Typed)
}
