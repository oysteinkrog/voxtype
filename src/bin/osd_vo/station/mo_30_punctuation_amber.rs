//! MO-30 Silence becomes punctuation on the amber look: pause ticks on the
//! transcript's baseline gather into a hollow comma, in the text's amber.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-30", "Silence becomes punctuation (amber)", OsdTheme::Amber)
        .feature("pause-marks")
        .moment(Moment::Pause)
}
