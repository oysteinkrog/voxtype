//! MO-81 on the orb look: the type breathes with the voice's loudness and pace.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable breath (orb)", OsdTheme::Orb)
        .feature("variable-breath")
        .moment(Moment::Dictation)
}
