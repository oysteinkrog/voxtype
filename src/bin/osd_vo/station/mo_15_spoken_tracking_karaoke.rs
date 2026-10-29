//! MO-15 spoken-time tracking on the karaoke look: a word on the prompter
//! arrives spaced by how long it took to say, then settles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-15", "Spoken-time tracking (karaoke)", OsdTheme::Karaoke)
        .feature(channels::TRACKING)
        .moment(Moment::Pause)
}
