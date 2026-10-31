//! MO-34 Overflow spine on the tron look: each line that leaves the top
//! leaves word-wide blocks along the text's top edge, which close into one
//! history tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-34", "Overflow spine: tron", OsdTheme::Tron)
        .feature("overflow-spine")
        .moment(Moment::Overflow)
}
