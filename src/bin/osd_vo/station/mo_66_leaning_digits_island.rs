//! MO-66. Leaning digits on the island look, under the clock.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-66", "Leaning digits (island)", OsdTheme::Island)
        .feature("lean-pace")
        .moment(Moment::Pause)
}
