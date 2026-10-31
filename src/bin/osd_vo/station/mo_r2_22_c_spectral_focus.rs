//! R2-22-c Spectral focus on the mission look: fine dust glows behind the
//! ghost tail, which stands at 70 % ink, and fades as its words commit.
//! The ghost tail is in every look's own set; the exhibit turns it on too.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-22-c", "Spectral focus: mission", OsdTheme::Mission)
        .feature("spectral-focus")
        .feature("ghost-tail")
        .moments(&[Moment::Lookahead, Moment::Dictation])
}
