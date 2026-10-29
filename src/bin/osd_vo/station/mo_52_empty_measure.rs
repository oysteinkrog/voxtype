//! MO-52 no speech: the breath line goes flat, two measuring marks close
//! on nothing and the caret types "No speech".

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-52", "No speech: the empty measure", OsdTheme::Type)
        .feature("empty-measure")
        .moment(Moment::NoSpeech)
}
