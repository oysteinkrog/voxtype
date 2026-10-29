//! MO-66. Leaning digits on the type look: the words-per-minute digits lean
//! up to 8 degrees with the speech rate and stand up in a pause.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-66", "Leaning digits", OsdTheme::Type)
        .feature("lean-pace")
        .moments(&[Moment::Pause, Moment::Dictation])
}
