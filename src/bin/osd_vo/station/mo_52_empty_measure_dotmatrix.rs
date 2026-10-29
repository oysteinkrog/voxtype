//! MO-52 no speech on the dotmatrix look: the breath line of dots goes
//! flat, two columns of dots close on nothing and the caret types
//! "No speech".

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-52", "No speech: the empty measure (dots)", OsdTheme::Dotmatrix)
        .feature("empty-measure")
        .moment(Moment::NoSpeech)
}
