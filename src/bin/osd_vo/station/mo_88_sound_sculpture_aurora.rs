//! MO-88 on the aurora look: on the centred newest line each letter stands
//! as deep as its word was loud; older lines lie flat.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-88", "Sound sculpture: aurora", OsdTheme::Aurora)
        .feature("sound-sculpture")
        .moment(Moment::Dictation)
}
