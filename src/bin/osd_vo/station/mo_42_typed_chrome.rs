//! MO-42 hand-off on the chrome look: the status icon turns into a spinner
//! while the final text is made, the waveform's bars settle into dots, and
//! an arrow leaves along it to the right as the text goes to the app, while
//! the chrome blob above the card settles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the text is sent (chrome)",
        OsdTheme::Chrome,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
