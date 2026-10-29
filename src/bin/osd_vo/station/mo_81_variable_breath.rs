//! MO-81 Variable font breathing on the swissfi look: each word in its own
//! Roboto Flex instance, weight from loudness, width from tempo and slant
//! from pitch, inside the reading-line envelope.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable font breathing", OsdTheme::Swissfi)
        .feature("variable-breath")
        .feature("no-pre-echo")
        .feature("no-pause-marks")
        .feature("no-exhale")
        .moments(&[Moment::Dictation, Moment::Pause])
}
