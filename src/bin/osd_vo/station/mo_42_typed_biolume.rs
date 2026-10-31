//! MO-42 hand-off on the biolume look: the bars in the jellyfish's bell
//! lower while the final text is made and settle when the words land, and
//! the status turns green. A cancel greys the bell and its tendrils.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the jellyfish hands over (biolume)",
        OsdTheme::Biolume,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
