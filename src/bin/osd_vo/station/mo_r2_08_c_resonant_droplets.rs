//! R2-08-c Resonant droplets on the glass look: at each phrase end a
//! droplet falls into the water below the text and sends calm rings.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-08-c", "Resonant droplets: glass", OsdTheme::Glass)
        .feature("resonant-droplets")
        .moments(&[Moment::Pause, Moment::Dictation])
}
