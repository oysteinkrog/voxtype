//! MO-42 hand-off on the orb look: the status icon turns into a spinner
//! while the final text is made, then the waveform collapses into a bright
//! hairline that flies out to the right as the text goes to the app, and
//! the aurora orb settles.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-42",
        "Hand-off: the hairline flies out (orb)",
        OsdTheme::Orb,
    )
    .moments(&[Moment::Typed, Moment::Cancel])
}
