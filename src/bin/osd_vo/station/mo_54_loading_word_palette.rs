//! MO-54 the status word is the loading bar, on the palette look.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-54",
        "The status word is the loading bar (palette)",
        OsdTheme::Palette,
    )
    .feature("loading-word")
    .moment(Moment::Loading)
}
