//! MO-35 time-limit hairline on the mission look: in the last 10 s before the
//! recording limit, a hairline under the newest line of the transcript tile
//! shortens from the right and turns red at 3 s. The tile sets a 12 s limit so
//! the scripted long recording reaches it.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-35", "Time-limit hairline (mission)", OsdTheme::Mission)
        .feature("limit-hairline")
        .feature("limit-secs-12")
        .moment(Moment::TimeLimit)
}
