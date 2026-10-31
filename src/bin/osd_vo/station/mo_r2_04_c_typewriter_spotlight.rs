//! R2-04-c Typewriter spotlight on the synthwave look: typewriter type, a
//! new word that lands 2 px low and settles, and a soft cone of light on it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-04-c",
        "Typewriter spotlight: synthwave",
        OsdTheme::Synthwave,
    )
    .feature("typewriter-spotlight")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
