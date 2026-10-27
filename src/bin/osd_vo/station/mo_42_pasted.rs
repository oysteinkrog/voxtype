//! MO-42 hand-off, pasted with Enter: the lines collapse into the reading
//! line, squash to a bright hairline that stamps down, then a return tick.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-42", "Hand-off: the text leaves (paste, Enter)", OsdTheme::Tron)
        .moment(Moment::PastedEnter)
}
