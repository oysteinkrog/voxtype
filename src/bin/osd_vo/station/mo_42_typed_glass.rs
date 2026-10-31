//! MO-42 hand-off on the glass look: the ring around the microphone turns
//! into a spinner while the final text is made, then the button shows a
//! check and the status reads the delivery and the word count as the text
//! leaves for the app.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the button checks (glass)",
        OsdTheme::Glass,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
