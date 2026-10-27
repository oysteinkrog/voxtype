//! MO-43 delivery receipt: the line lengths fold into a small notch on the
//! reading line beside what the daemon reported (typed or pasted, the app,
//! Enter).

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-43", "Delivery receipt", OsdTheme::Palette)
        .moments(&[Moment::PastedEnter, Moment::Typed])
}
