//! MO-12 Exhale and snap back on the swissfi look: after 3 s of quiet the
//! chrome rests; the next onset snaps it back before the word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-12", "Exhale and snap back", OsdTheme::Swissfi)
        .feature("exhale")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .moment(Moment::LongSilence)
}
