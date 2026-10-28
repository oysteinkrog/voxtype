//! MO-83 Letter physics as a specimen: the newest word falls freely from
//! well above its line with motion vectors, over the reading line where the
//! same physics keeps within the 3 px envelope.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-83", "Letter physics: specimen", OsdTheme::Swissfi)
        .view(super::wild_motion::letter_specimen)
        .moments(&[Moment::Dictation, Moment::Pause])
}
