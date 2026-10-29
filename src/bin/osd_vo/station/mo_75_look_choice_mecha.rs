//! MO-75 per-look choice, mecha: with `look-choice` a cancel winds the words
//! back into the caret, and the top line folds under.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-75", "Per-look choice: mecha", OsdTheme::Mecha)
        .feature("look-choice")
        .moments(&[Moment::Cancel, Moment::Overflow])
}
