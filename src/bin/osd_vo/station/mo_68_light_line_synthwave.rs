//! MO-68 on the synthwave look: the tube's light leaves the frame to underline each new word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-68", "Light line under words (synthwave)", OsdTheme::Synthwave)
        .feature("light-line")
        .moment(Moment::Pause)
}
