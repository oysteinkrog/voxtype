//! MO-83 Letter physics on the swissfi look: new letters drop into the line,
//! bounce, nudge their neighbours and lock into place, with faint motion
//! vectors while they fly.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-83", "Letter physics", OsdTheme::Swissfi)
        .feature("letter-physics")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
