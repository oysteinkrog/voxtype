//! EG-16 Line wave on the swissfi look: the older lines ride a 3 px wave,
//! each letter turned to follow its slope, while the newest line holds
//! still for reading.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("EG-16", "Line wave", OsdTheme::Swissfi)
        .feature(super::wild_motion::LINE_WAVE)
        .moments(&[Moment::Dictation, Moment::Pause])
}
