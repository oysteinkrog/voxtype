//! MO-88 on the chrome look: on the newest line each letter stands as deep as its word
//! was loud; older lines lie flat.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-88", "Sound sculpture: chrome", OsdTheme::Chrome)
        .feature("sound-sculpture")
        .moment(Moment::Dictation)
}
