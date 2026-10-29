//! MO-14 Pitch baseline on the swiss look: the newest word's baseline follows
//! the pitch by up to 3 px while it is said (the pitch rises before the pause)
//! and relaxes flat over 600 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-14", "Pitch baseline", OsdTheme::Swiss)
        .feature("pitch-baseline")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::Pause)
}
