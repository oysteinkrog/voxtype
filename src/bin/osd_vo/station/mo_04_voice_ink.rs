//! MO-04 signal-to-glyph binding on the type look: onset, pitch and the
//! time a word took to say reach its weight, baseline and tracking.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-04", "Signal-to-glyph binding", OsdTheme::Type)
        .feature(channels::VOICE)
        .moment(Moment::Dictation)
}
