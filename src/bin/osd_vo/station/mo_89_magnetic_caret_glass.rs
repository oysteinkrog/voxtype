//! MO-89 Magnetic caret on the glass look: new words snap to the caret at
//! the end of the centred newest line, its field follows the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-89", "Magnetic caret (glass)", OsdTheme::Glass)
        .feature("magnetic-caret")
        .moments(&[Moment::Pause, Moment::LongSilence])
}
