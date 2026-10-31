//! MO-88 sound sculpture on the river look: on the reading line each letter
//! stands as deep as its word was loud; older lines lie flat.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-88", "Sound sculpture: river", OsdTheme::River)
        .feature("sound-sculpture")
        .moment(Moment::Dictation)
}
