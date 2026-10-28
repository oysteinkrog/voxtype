//! MO-35. The time-limit hairline on the swissfi look: in the last 10 s
//! before the recording limit, a hairline under the reading line shortens
//! from the right at the real rate and changes colour at 3 s. The tile sets
//! a 12 s limit so the scripted long recording reaches it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-35", "Time-limit hairline", OsdTheme::Swissfi)
        .feature("limit-hairline")
        .feature("limit-secs-12")
        .moment(Moment::TimeLimit)
}
