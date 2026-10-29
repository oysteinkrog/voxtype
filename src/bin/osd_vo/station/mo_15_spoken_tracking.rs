//! MO-15 spoken-time tracking on the type look: a word arrives spaced by
//! how long it took to say, then settles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-15", "Spoken-time tracking", OsdTheme::Type)
        .feature(channels::TRACKING)
        .moment(Moment::Pause)
}
