//! MO-83 letter physics on the particles look: new letters drop into the centred
//! line, bounce and lock, while the word forms from particles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-83", "Letter physics (particles)", OsdTheme::Particles)
        .feature(wild_motion::LETTER_PHYSICS)
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
