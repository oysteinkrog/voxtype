//! MO-03 per-word typographic channels on the type look: each new word
//! arrives with every channel moved (fill, weight, tracking, baseline) and
//! settles to plain type, inside the reading-line envelope.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-03", "Per-word typographic channels", OsdTheme::Type)
        .feature(channels::CHANNELS)
        .moment(Moment::Dictation)
}
