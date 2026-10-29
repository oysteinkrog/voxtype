//! MO-85 kinetic lattice on the swiss look (LT-07), in the hairline style:
//! strands in a gutter under the words, bow by the pause before a word,
//! weight by how loud it was, colour by its pitch, and a live strand to where
//! the next word starts while it listens.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-85", "Kinetic lattice (Hairline)", OsdTheme::Swiss)
        .feature(wild_space::KINETIC_LATTICE)
        .moments(&[Moment::Lattice, Moment::Pause, Moment::RevisionBurst])
}
