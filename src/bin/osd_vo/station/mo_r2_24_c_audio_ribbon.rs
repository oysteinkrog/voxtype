//! R2-24-c Audio ribbon on the river look: a faint ribbon of light behind
//! the words swings with the voice and flattens into a thin still line over
//! 600 ms once the dictation stops.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-24-c", "Audio ribbon: river", OsdTheme::River)
        .feature("audio-ribbon")
        .moments(&[Moment::Dictation, Moment::FinalPass])
}
