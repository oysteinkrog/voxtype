//! MO-41 remaining audio as a ruler on the mission look: while finishing, the
//! status bar shows the seconds of audio left as orange units that close into
//! hairlines and join at the end, in place of the progress line.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-41",
        "Remaining audio as a ruler (mission)",
        OsdTheme::Mission,
    )
    .feature("audio-ruler")
    .moment(Moment::FinalPass)
}
