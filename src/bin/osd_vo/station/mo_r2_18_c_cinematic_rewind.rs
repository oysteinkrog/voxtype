//! R2-18-c Cinematic rewind on the scope look: each word becomes a tile on
//! a strip under the text, sliding left and shrinking as newer words come.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-18-c", "Cinematic rewind: scope", OsdTheme::Scope)
        .feature("cinematic-rewind")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
