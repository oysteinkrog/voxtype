//! MO-42 hand-off on the aurora look: the line of light lowers while the
//! final text is made and flattens when the words land, and the status
//! turns green. A cancel greys the mesh and the rim.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the aurora card hands over",
        OsdTheme::Aurora,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
