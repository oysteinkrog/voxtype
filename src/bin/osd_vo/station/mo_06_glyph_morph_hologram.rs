//! MO-06 letter outlines as paths on the hologram look: a new word's
//! outlines grow out of a dot under it and a revised word's out of a line;
//! a cancel folds them back into the line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-06",
        "Glyph outlines as paths (hologram)",
        OsdTheme::Hologram,
    )
    .feature("glyph-morph")
    .moments(&[Moment::RevisionBurst, Moment::Cancel])
}
