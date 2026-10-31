//! MO-73 onset joint on the radial look: each onset opens a 2 px joint in
//! the foot of a caret after the newest word, and the word it starts gets
//! that joint as a baseline tick for 120 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-73", "Onset joint: radial", OsdTheme::Radial)
        .feature(sounding::ONSET_JOINT)
        .moments(&[Moment::FirstWord, Moment::Pause])
}
