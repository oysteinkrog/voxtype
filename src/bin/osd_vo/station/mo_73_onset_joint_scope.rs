//! MO-73 onset joint on the scope look: the caret's foot splits at each
//! onset and the word it starts gets a baseline tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-73", "Onset joint", OsdTheme::Scope)
        .feature(sounding::ONSET_JOINT)
        .moments(&[Moment::FirstWord, Moment::Pause])
}
