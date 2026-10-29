//! MO-24 confidence ink on the type look: tentative words at 75 % ink over
//! an open seam; after two unchanged hypotheses they set and the seam closes.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-24", "Confidence as ink, stability seam", OsdTheme::Type)
        .feature(channels::CONFIDENCE)
        .moment(Moment::RevisionBurst)
}
