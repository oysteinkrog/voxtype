//! R2-19-b Luminous ink on the aurora look: each word blooms a dye cloud behind
//! it, and the dye thins out within 2 s of silence.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-19-b", "Luminous ink: aurora", OsdTheme::Aurora)
        .feature("luminous-ink")
        .moments(&[Moment::Dictation, Moment::Pause])
}
