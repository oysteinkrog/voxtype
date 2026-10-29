//! MO-02 delivery events: the popup reads `result.json` at delivery (method,
//! Enter, timing, target app). The type look shows the delivery it read.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-02", "Delivery events in the popup", OsdTheme::Type)
        .moments(&[Moment::Typed, Moment::PastedEnter])
}
