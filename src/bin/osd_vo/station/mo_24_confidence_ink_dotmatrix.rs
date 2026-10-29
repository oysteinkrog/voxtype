//! MO-24 confidence ink on the dotmatrix look: tentative words at 75 % ink
//! over a row of dots with a gap; after two unchanged hypotheses they set,
//! the gap fills and the dots fade.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-24", "Confidence as ink, seam of dots", OsdTheme::Dotmatrix)
        .feature(channels::CONFIDENCE)
        .moment(Moment::RevisionBurst)
}
