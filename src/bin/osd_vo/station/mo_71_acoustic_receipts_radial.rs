//! MO-71 acoustic receipts on the radial look: under the newest three words,
//! the sound that made each word folds into a short underline, then into a
//! timestamp tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-71", "Acoustic receipts: radial", OsdTheme::Radial)
        .feature(sounding::ACOUSTIC_RECEIPTS)
        .moments(&[Moment::Dictation, Moment::Pause])
}
