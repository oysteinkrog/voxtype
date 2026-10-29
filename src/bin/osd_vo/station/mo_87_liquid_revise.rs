//! MO-87 as a live-text revision (EG-32), on the type look: in a burst of
//! revisions each corrected word flows into its new letters as liquid, in the
//! text's own colour; kept letters stay solid and the reading line stays
//! readable.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-87", "Liquid revision (EG-32)", OsdTheme::Type)
        .feature(revision::LIQUID)
        .moment(Moment::RevisionBurst)
}
