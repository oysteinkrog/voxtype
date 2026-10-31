//! MO-42 hand-off on the river look: the ribbon of light calms while the
//! final text is made, then flattens into one bright hairline that ends in
//! an arrow as the text leaves for the app.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the river flattens (river)",
        OsdTheme::River,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
