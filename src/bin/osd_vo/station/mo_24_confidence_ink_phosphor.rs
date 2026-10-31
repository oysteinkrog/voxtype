//! MO-24 confidence ink on the phosphor look: tentative words at 75 % ink
//! over a row of green phosphor dots with a gap; after two unchanged
//! hypotheses they set, the gap fills and the dots fade.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-24", "Confidence as ink: phosphor", OsdTheme::Phosphor)
        .feature(channels::CONFIDENCE)
        .moment(Moment::RevisionBurst)
}
