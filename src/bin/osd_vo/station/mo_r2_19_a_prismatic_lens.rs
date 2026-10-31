//! R2-19-a Prismatic lens on the aurora look: soft rainbow caustics flow round the
//! card's rim while the voice moves them.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-19-a", "Prismatic lens: aurora", OsdTheme::Aurora)
        .feature("prismatic-lens")
        .moments(&[Moment::Dictation, Moment::Pause])
}
