//! MO-20 palimpsest correction, the default revision in every look: in a burst of
//! revisions the letters that survive stay, removed ones sink into a short ghost and new
//! ones arrive in place.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-20", "Palimpsest correction", OsdTheme::Swiss).moment(Moment::RevisionBurst)
}
