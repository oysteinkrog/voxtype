//! MO-83 letter physics on the hologram look: new letters drop into the
//! centred projected line, bounce and lock.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-83", "Letter physics (hologram)", OsdTheme::Hologram)
        .feature(wild_motion::LETTER_PHYSICS)
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
