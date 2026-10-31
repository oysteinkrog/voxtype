//! MO-81 on the aurora look: the type breathes with the voice's loudness and pace.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-81", "Variable breath (aurora)", OsdTheme::Aurora)
        .feature("variable-breath")
        .moment(Moment::Dictation)
}
