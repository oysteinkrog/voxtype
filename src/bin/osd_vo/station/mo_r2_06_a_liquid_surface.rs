//! R2-06-a Liquid surface on the phosphor look: the block meter becomes a
//! smooth flowing wave with a few particles riding it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-06-a", "Liquid surface: phosphor", OsdTheme::Phosphor)
        .feature("liquid-surface")
        .moments(&[Moment::Dictation, Moment::Pause])
}
