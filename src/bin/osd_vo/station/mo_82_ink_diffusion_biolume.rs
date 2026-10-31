//! MO-82 on the biolume look: ink spreads through the newest letters as they land.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-82", "Ink diffusion (biolume)", OsdTheme::Biolume)
        .feature("ink-diffusion")
        .moment(Moment::Dictation)
}
