//! R2-11-c Typewriter horizon on the cyberpunk look: a platen roller under
//! the newest line turns a notch per word and a carriage marker glides to
//! the newest word; the text stays still.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "R2-11-c",
        "Typewriter horizon: cyberpunk",
        OsdTheme::Cyberpunk,
    )
    .feature("typewriter-horizon")
    .moments(&[Moment::FirstWord, Moment::Dictation])
}
