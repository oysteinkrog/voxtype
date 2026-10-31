//! R2-17-b Typewriter reveal on the palette look: when a line wraps the
//! older lines fade to a soft ghost and the newest line stays crisp.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-17-b", "Typewriter reveal: palette", OsdTheme::Palette)
        .feature("typewriter-reveal")
        .moments(&[Moment::Wrap, Moment::Dictation])
}
