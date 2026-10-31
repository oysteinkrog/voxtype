//! R2-02-a Living glass on the jarvis look: a glass bubble over the inner
//! ring bends the ring and the spectrum under it by at most 3 px, its
//! ripple following the voice. It bends only what the popup drew.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-02-a", "Living glass: jarvis", OsdTheme::Jarvis)
        .feature("living-glass")
        .moments(&[Moment::Dictation, Moment::Pause])
}
