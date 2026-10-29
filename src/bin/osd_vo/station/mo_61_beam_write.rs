//! MO-61. Beam-write on the phosphor look: a bright 3 px beam head writes
//! each word in 80 ms, and a revision rewrites only the changed letters.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-61", "Beam-write", OsdTheme::Phosphor)
        .feature("beam-write")
        .moments(&[Moment::RevisionBurst, Moment::Dictation])
}
