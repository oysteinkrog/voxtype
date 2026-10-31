//! MO-85 kinetic lattice on the river look: light strands under the words
//! breathe with the voice and show the pauses between them.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-85", "Kinetic lattice: river", OsdTheme::River)
        .feature("kinetic-lattice")
        .moment(Moment::Pause)
}
