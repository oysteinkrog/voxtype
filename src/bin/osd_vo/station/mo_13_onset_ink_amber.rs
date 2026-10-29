//! MO-13 Onset ink on the amber look: a word lands heavier on a hard onset
//! and thins over 180 ms; a soft start arrives light and fills in.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-13", "Onset ink (amber)", OsdTheme::Amber)
        .feature("onset-ink")
        .moment(Moment::Pause)
}
