//! R2-03-b Ink flow on the island look: soft blue ink behind the text, fed
//! at the caret by the voice and drifting back under the older words; it is
//! gone 2 s after the voice stops.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-03-b", "Ink flow: island", OsdTheme::Island)
        .feature("ink-flow")
        .moments(&[Moment::Dictation, Moment::Pause])
}
