//! MO-80 stroke-order writing on the river look: the pen writes each new
//! word under the ribbon of light.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: river", OsdTheme::River)
        .feature("stroke-write")
        .moment(Moment::Dictation)
}
