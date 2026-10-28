//! MO-72 pitch margin score on the karaoke look: beside the line, up to six
//! marks place the phrase's pitch and join into a contour.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-72", "Pitch margin score", OsdTheme::Karaoke)
        .feature(sounding::PITCH_SCORE)
        .moments(&[Moment::Dictation, Moment::Pause])
}
