//! MO-68 light line under words on tron: at each onset the border's light
//! head leaves the border, draws a line under the new word and goes back.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-68", "Light line under words", OsdTheme::Tron)
        .feature(tron::LIGHT_LINE)
        .moments(&[Moment::Pause, Moment::FirstWord])
}
