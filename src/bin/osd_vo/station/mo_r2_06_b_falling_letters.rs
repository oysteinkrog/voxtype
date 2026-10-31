//! R2-06-b Falling letters on the phosphor look: faint copies of a new
//! word's letters fall into place in the leading above it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-06-b", "Falling letters: phosphor", OsdTheme::Phosphor)
        .feature("falling-letters")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
