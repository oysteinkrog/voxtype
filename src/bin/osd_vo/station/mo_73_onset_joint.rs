//! MO-73 onset joint on the mecha look: each onset opens a 2 px joint in
//! the caret's foot, and the word it starts gets that joint as a baseline
//! tick for 120 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-73", "Onset joint", OsdTheme::Mecha)
        .feature(mecha::ONSET_JOINT)
        .moments(&[Moment::FirstWord, Moment::Pause])
}
