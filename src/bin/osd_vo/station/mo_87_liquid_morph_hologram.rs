//! MO-87 glyph liquid morph on the hologram look: a revised word flows
//! into its correction; shared letters stay solid, changed ones blend as
//! liquid.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-87", "Glyph liquid morph (hologram)", OsdTheme::Hologram)
        .feature(letter_fx::LIQUID)
        .moment(Moment::RevisionBurst)
}
