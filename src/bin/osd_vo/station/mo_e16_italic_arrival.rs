//! EG-16 Italic arrival on the swissfi look: each new word arrives leaning
//! 5 degrees and straightens in 0.2 s.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("EG-16", "Italic arrival", OsdTheme::Swissfi)
        .feature(super::wild_motion::ITALIC_ARRIVAL)
        .feature("no-pre-echo")
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
