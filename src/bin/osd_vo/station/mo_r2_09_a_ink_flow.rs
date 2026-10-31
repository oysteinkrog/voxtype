//! R2-09-a Ink flow on the particles look: soft ink round the button and a
//! thin ribbon of it behind the text, stirred by the voice; the newest word
//! blooms from 70 % to full ink in 300 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-09-a", "Ink flow: particles", OsdTheme::Particles)
        .feature("ink-flow")
        .moments(&[Moment::Dictation, Moment::Pause])
}
