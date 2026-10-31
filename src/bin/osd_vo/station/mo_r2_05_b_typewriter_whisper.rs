//! R2-05-b Typewriter whisper on the chrome look: fine letter dust rises
//! to the newest word from just under it and fades as it arrives within
//! 400 ms, and the word keeps a faint halo for 500 ms.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-05-b", "Typewriter whisper: chrome", OsdTheme::Chrome)
        .feature("typewriter-whisper")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
