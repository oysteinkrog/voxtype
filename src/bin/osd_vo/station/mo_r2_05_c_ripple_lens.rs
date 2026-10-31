//! R2-05-c Ripple lens on the chrome look: a ring runs over the chrome's
//! dome for each syllable, and a ring spreads on a strip of water under the
//! words for each new word; at most four of each at once.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-05-c", "Ripple lens: chrome", OsdTheme::Chrome)
        .feature("ripple-lens")
        .moments(&[Moment::Dictation, Moment::Pause])
}
