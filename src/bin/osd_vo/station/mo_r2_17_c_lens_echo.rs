//! R2-17-c Lens echo on the palette look: each new word sends wide arcs to
//! the right behind the text, in a fading trail.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-17-c", "Lens echo: palette", OsdTheme::Palette)
        .feature("lens-echo")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
