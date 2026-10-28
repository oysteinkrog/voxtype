//! MO-75 per-look choice, cyberpunk: with `look-choice` the look takes its own
//! cancel and overflow. A cancel winds the words back into a yellow caret
//! before the red strike; the top line folds under.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-75", "Per-look choice: cyberpunk", OsdTheme::Cyberpunk)
        .feature("look-choice")
        .moments(&[Moment::Cancel, Moment::Overflow])
}
