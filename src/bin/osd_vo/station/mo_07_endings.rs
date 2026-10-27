//! MO-07 moment scenarios: the three endings that linger (cancel, no
//! speech, failure), in turn, on jarvis. Copy this file to add an exhibit.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-07",
        "Moments: cancel, no speech, failure",
        OsdTheme::Jarvis,
    )
    .moments(&[Moment::Cancel, Moment::NoSpeech, Moment::Failure])
}
