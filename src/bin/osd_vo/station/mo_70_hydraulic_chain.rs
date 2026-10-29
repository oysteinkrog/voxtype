//! MO-70 hydraulic chain on the mecha look: a new word pushes the caret and,
//! two links later, the status plate; a revision slides the word's plate out
//! and back.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-70", "Hydraulic chain", OsdTheme::Mecha)
        .feature(mecha::HYDRAULIC_CHAIN)
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
