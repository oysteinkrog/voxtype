//! R2-09-b Prism reveal on the particles look: a beam enters a small glass
//! prism left of the button and leaves as a spectrum fan that opens with the
//! voice; the newest word starts in the spectrum and goes back to its own
//! colours over 600 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-09-b", "Prism reveal: particles", OsdTheme::Particles)
        .feature("prism-reveal")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
