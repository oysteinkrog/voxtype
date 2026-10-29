//! MO-50 rewind cancel on the particles look: the words go back into the caret,
//! newest first, and the swirl's voice goes back over the same audio.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-50", "Rewind cancel (particles)", OsdTheme::Particles)
        .feature("rewind")
        .moments(&[Moment::Cancel])
}
