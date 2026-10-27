//! MO-06 letter outlines as paths: on the type look, a new word's outlines
//! grow out of a dot under it and a revised word's out of a line, as an
//! accent stroke over the word; a cancel folds them back into the line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-06", "Glyph outlines as paths", OsdTheme::Type)
        .feature("glyph-morph")
        .moments(&[Moment::RevisionBurst, Moment::Cancel])
}
