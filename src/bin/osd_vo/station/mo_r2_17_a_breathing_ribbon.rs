//! R2-17-a Breathing ribbon on the palette look: a silk ribbon behind the
//! bar waveform folds and unfolds with the voice.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-17-a", "Breathing ribbon: palette", OsdTheme::Palette)
        .feature("breathing-ribbon")
        .moments(&[Moment::Dictation, Moment::Pause])
}
