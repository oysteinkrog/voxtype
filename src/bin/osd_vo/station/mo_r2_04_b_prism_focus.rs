//! R2-04-b Prism focus on the synthwave look: the waveform runs into a
//! small glass prism and leaves it as five spectrum ribbons, longer and
//! brighter with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-04-b", "Prism focus: synthwave", OsdTheme::Synthwave)
        .feature("prism-focus")
        .moments(&[Moment::Dictation, Moment::Pause])
}
