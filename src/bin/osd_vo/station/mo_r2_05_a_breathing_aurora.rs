//! R2-05-a Breathing aurora on the chrome look: an aurora glows inside
//! the see-through chrome, brighter with the voice, drifting slowly in a
//! pause.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-05-a", "Breathing aurora: chrome", OsdTheme::Chrome)
        .feature("breathing-aurora")
        .moments(&[Moment::Dictation, Moment::Pause])
}
