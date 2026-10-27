//! MO-07 moment scenarios: the final pass that puts three words right, then
//! the wrap and the overflow, on swiss.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new(
        "MO-07",
        "Moments: final pass, wrap, overflow",
        OsdTheme::Swiss,
    )
    .moments(&[Moment::FinalPass, Moment::Wrap, Moment::Overflow])
}
