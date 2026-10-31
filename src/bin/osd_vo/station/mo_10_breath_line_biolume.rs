//! MO-10 on the biolume look: a line of breath draws the first word into its seat.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-10", "Breath line (biolume)", OsdTheme::Biolume)
        .feature("breath-line")
        .moment(Moment::FirstWord)
}
