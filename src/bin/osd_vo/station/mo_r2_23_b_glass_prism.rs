//! R2-23-b Glass prism on the mecha look: the text sits on a chamfered
//! glass slab, and a faint spectrum band sweeps across it once per word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-23-b", "Glass prism: mecha", OsdTheme::Mecha)
        .feature("glass-prism")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
