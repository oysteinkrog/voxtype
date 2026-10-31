//! R2-19-c Magnetic type on the aurora look: faint field lines curve with the voice
//! and the older lines ride the same curve.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-19-c", "Magnetic type: aurora", OsdTheme::Aurora)
        .feature("magnetic-type")
        .moments(&[Moment::Dictation, Moment::Wrap])
}
