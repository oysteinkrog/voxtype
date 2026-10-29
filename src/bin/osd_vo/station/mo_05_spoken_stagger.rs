//! MO-05 spoken-time staggers on the type look: words that come in one
//! hypothesis settle in spoken order at a quarter of the spoken rhythm
//! (200 to 600 ms in all).

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-05", "Spoken-time staggers", OsdTheme::Type)
        .feature(channels::STAGGER)
        .moment(Moment::RevisionBurst)
}
