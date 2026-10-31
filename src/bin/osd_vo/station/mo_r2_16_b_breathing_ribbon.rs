//! R2-16-b Breathing ribbon on the tron look: faint strands of light drift
//! across the waveform's band and swell with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-16-b", "Breathing ribbon: tron", OsdTheme::Tron)
        .feature("breathing-ribbon")
        .moments(&[Moment::Dictation, Moment::Pause])
}
