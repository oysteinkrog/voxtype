//! R2-03-c Tape reveal on the island look: sprocket holes on the bottom
//! edge step one hole per word, and an amber glow sweeps behind the new
//! word as its ink comes up from 70 %. The text does not move.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-03-c", "Tape reveal: island", OsdTheme::Island)
        .feature("tape-reveal")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
