//! R2-03-a Breathing lens on the island look: four thin lens rims between
//! the waveform and the text breathe 0.97 to 1.03 with the voice; each word
//! lights the next rim and sends an arc round it that fades in 600 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-03-a", "Breathing lens: island", OsdTheme::Island)
        .feature("breathing-lens")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
