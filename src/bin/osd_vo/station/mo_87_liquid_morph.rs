//! MO-87 glyph liquid morph on the type look: a revised word flows into its
//! correction; shared letters stay solid, changed ones blend as liquid.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-87", "Glyph liquid morph", OsdTheme::Type)
        .feature(letter_fx::LIQUID)
        .moment(Moment::RevisionBurst)
}
