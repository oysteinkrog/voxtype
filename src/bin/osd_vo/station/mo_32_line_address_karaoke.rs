//! MO-32 The line changes its address, on the karaoke look: a cyan mark beside
//! the newest line; when a new line starts, its top half rolls up and fades
//! with the line before, and the new line keeps the mark.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-32", "The line changes its address (karaoke)", OsdTheme::Karaoke)
        .feature("margin-mark")
        .moment(Moment::Wrap)
}
