//! MO-20 Palimpsest correction on the cyberpunk look: kept letters stay,
//! removed ones sink into a faint ghost while a glitch burst shakes the panel.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-20",
        "Palimpsest correction: cyberpunk",
        OsdTheme::Cyberpunk,
    )
    .moment(Moment::RevisionBurst)
}
