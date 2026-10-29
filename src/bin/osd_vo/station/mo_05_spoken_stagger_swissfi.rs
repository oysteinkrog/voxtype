//! MO-05 spoken-time staggers on the swissfi look: the words of one hypothesis
//! rise 2 px into place in spoken order, from 80 % ink.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-05", "Spoken-time staggers", OsdTheme::Swissfi)
        .feature(channels::STAGGER)
        .feature("no-count-cast")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::RevisionBurst)
}
