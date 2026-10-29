//! MO-13 Onset ink on the jarvis look: the newest line's words land heavier
//! on a hard onset and thin over 180 ms; a soft start arrives light and fills
//! in.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-13", "Onset ink (jarvis)", OsdTheme::Jarvis)
        .feature("onset-ink")
        .moment(Moment::Pause)
}
