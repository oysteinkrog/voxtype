//! MO-43 delivery receipt on the island look: the text leaves for the app
//! and the receipt folds into the pill.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-43", "Delivery receipt (island)", OsdTheme::Island)
        .moments(&[Moment::PastedEnter, Moment::Typed])
}
