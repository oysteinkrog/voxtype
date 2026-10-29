//! MO-20 palimpsest correction on the swissfi look, with its correction hinge
//! and revision address: removed letters sink into a short ghost and new ones
//! arrive in place.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-20", "Palimpsest correction", OsdTheme::Swissfi)
        .feature("no-spoken-stagger")
        .feature("no-count-cast")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::RevisionBurst)
}
