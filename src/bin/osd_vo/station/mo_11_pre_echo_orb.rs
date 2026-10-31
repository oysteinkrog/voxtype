//! MO-11 on the orb look: syllable blobs grow with the voice before the first words show.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-11", "Pre-echo (orb)", OsdTheme::Orb)
        .feature("pre-echo")
        .moment(Moment::FirstWord)
}
