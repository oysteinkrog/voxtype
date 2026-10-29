//! MO-89 Magnetic caret on the swissfi look: new words snap to the caret,
//! its field follows the voice and relaxes into rings in silence.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-89", "Magnetic caret", OsdTheme::Swissfi)
        .feature("magnetic-caret")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moments(&[Moment::Pause, Moment::LongSilence])
}
