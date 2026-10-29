//! MO-54 the status word is the loading bar: "Loading" fills letter by
//! letter with the real progress, then turns into "Listening".

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-54",
        "The status word is the loading bar",
        OsdTheme::Type,
    )
    .feature("loading-word")
    .moment(Moment::Loading)
}
