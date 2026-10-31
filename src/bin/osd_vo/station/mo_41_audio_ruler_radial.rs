//! MO-41 remaining audio as a ruler on the radial look: while finishing, the
//! status row ends in the seconds of audio left as units that close into
//! hairlines and join at the end.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-41",
        "Remaining audio as a ruler: radial",
        OsdTheme::Radial,
    )
    .feature("audio-ruler")
    .moment(Moment::FinalPass)
}
