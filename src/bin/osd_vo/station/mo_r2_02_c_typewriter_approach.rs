//! R2-02-c Typewriter approach on the jarvis look: when a word comes, two
//! to four faint copies of it glide round the ring towards it and fade out
//! above the frame within 300 ms. The word comes in at 70 % ink, in place.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("R2-02-c", "Typewriter approach: jarvis", OsdTheme::Jarvis)
        .feature("typewriter-approach")
        .moments(&[Moment::FirstWord, Moment::Dictation])
}
