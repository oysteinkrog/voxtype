//! MO-40 The final pass sets the type, on jarvis.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-40", "The final pass sets the type", OsdTheme::Jarvis).moment(Moment::FinalPass)
}
