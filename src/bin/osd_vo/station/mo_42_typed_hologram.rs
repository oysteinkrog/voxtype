//! MO-42 hand-off on the hologram look: the wave at the column's base
//! lowers while the final text is made, then the words leave the light for
//! the app and the ring says where they went. A cancel greys the light.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the projection hands over (hologram)",
        OsdTheme::Hologram,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
