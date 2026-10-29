//! MO-12 Exhale and snap back on the island look: after 3 s of quiet the
//! wave goes flat, the clock greys and the rim light dims; the text stays.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-12", "Exhale and snap back (island)", OsdTheme::Island)
        .feature("exhale")
        .moment(Moment::LongSilence)
}
