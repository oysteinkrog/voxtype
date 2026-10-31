//! MO-81 on the biolume look: the type breathes with the voice's loudness and pace.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable breath (biolume)", OsdTheme::Biolume)
        .feature("variable-breath")
        .moment(Moment::Dictation)
}
