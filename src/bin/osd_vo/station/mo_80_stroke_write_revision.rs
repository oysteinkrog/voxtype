//! MO-80 stroke writing through a burst of revisions: the pen writes again
//! only the letters each revision changed, and the rest keep full ink.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: revision", OsdTheme::Type)
        .feature("stroke-write")
        .moment(Moment::RevisionBurst)
}
