//! MO-82 on the chrome look: each new word spreads into the card like ink.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-82", "Ink diffusion (chrome)", OsdTheme::Chrome)
        .feature("ink-diffusion")
        .moment(Moment::Dictation)
}
