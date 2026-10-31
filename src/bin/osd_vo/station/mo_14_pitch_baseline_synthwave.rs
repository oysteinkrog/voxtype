//! MO-14 on the synthwave look: the newest word's baseline follows the pitch.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-14", "Pitch baseline (synthwave)", OsdTheme::Synthwave)
        .feature("pitch-baseline")
        .moment(Moment::Pause)
}
