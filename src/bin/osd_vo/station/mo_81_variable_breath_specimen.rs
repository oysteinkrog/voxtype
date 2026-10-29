//! MO-81 Variable font breathing as a type specimen: the newest word in the
//! full axis ranges its voice gives it, a readout per axis, and a matrix of
//! the same word in fixed axis states with the nearest cell lit.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable font breathing: specimen", OsdTheme::Swissfi)
        .view(super::wild_motion::breath_specimen)
        .moments(&[Moment::Dictation, Moment::Pause])
}
