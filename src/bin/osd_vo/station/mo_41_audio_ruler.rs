//! MO-41 remaining audio as a ruler: while finishing, swiss's footer shows the
//! seconds of audio left as units that close into hairlines and join at the end.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-41", "Remaining audio as a ruler", OsdTheme::Swiss)
        .feature("audio-ruler")
        .moment(Moment::FinalPass)
}
