//! MO-75 per-look choice, swissfi: with `look-choice` a thread pulls back
//! under the words on a cancel, and the newest line's bar moves on a wrap.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-75", "Per-look choice: swissfi", OsdTheme::Swissfi)
        .feature("look-choice")
        .moments(&[Moment::Cancel, Moment::Wrap])
}
