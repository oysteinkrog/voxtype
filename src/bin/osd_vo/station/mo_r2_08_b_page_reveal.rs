//! R2-08-b Page reveal on the glass look: a paper strip behind the newest
//! line grows by each new word, its right end curled.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-08-b", "Page reveal: glass", OsdTheme::Glass)
        .feature("page-reveal")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
