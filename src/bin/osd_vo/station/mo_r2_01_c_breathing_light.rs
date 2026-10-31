//! R2-01-c Breathing light on the orb look: the aurora inside the orb
//! grows and brightens with the smoothed voice, and its light spills
//! faintly behind the newest line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-01-c", "Breathing light: orb", OsdTheme::Orb)
        .feature("breathing-light")
        .moments(&[Moment::Dictation, Moment::Pause])
}
