//! MO-85 kinetic lattice on the tron look (LT-07), in the light style:
//! strands in a gutter under the words, bow by the pause before a word,
//! weight by how loud it was, colour by its pitch, and a live strand to where
//! the next word starts while it listens.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-85", "Kinetic lattice (Light)", OsdTheme::Tron)
        .feature(wild_space::KINETIC_LATTICE)
        .moments(&[Moment::Lattice, Moment::Pause, Moment::RevisionBurst])
}
