//! MO-11 pre-echo on the particles look: faint blobs per onset where the next word
//! goes, before it comes, each turning into its word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-11", "Pre-echo caret (particles)", OsdTheme::Particles)
        .feature(hearing::PRE_ECHO)
        .moments(&[Moment::FirstWord])
}
