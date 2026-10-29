//! MO-85 kinetic lattice on the scope look: strands between the words, bow
//! by the pause before a word, weight by how loud it was, pulled taut as
//! each word arrives.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-85", "Kinetic lattice", OsdTheme::Scope)
        .feature(wild_space::KINETIC_LATTICE)
        .moments(&[Moment::Lattice, Moment::Pause, Moment::RevisionBurst])
}
