//! EG-16 Letter tumble on the swissfi look: a copy of each new letter flies
//! in from up to 40 px, turning, and lands on it within 0.35 s, while the
//! letter itself stands readable from its first frame.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("EG-16", "Letter tumble", OsdTheme::Swissfi)
        .feature(super::wild_motion::LETTER_TUMBLE)
        .feature("no-pre-echo")
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
