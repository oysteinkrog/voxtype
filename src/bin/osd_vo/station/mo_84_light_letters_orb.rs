//! MO-84 on the orb look: light shines through the letters onto the glass behind them.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-84", "Light letters (orb)", OsdTheme::Orb)
        .feature("light-letters")
        .moment(Moment::Dictation)
}
