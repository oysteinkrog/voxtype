//! MO-14 Pitch baseline on the captions look: the newest word's baseline
//! follows the pitch by up to 3 px while it is said and relaxes flat over
//! 600 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-14", "Pitch baseline (captions)", OsdTheme::Captions)
        .feature("pitch-baseline")
        .moment(Moment::Pause)
}
