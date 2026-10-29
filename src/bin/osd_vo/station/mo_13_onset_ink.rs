//! MO-13 Onset ink on the type look: a word born on a hard onset lands up to
//! 0.6 px heavier and thins over 180 ms; a soft start arrives light and fills
//! in.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-13", "Onset ink", OsdTheme::Type)
        .feature("onset-ink")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moment(Moment::Pause)
}
