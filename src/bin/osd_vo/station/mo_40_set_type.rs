//! MO-40 The final pass sets the type, on the type look: a soft front follows
//! the finishing fraction, spoken ink ahead and set type behind; when the
//! final text comes, the changed words change at the front with a nod and a
//! bracket.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-40", "The final pass sets the type", OsdTheme::Type).moment(Moment::FinalPass)
}
