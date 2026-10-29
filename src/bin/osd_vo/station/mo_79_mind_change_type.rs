//! MO-79 mind change on the type look (ASR-06): a replaced guess lingers 300 ms, and recent changes tint the edge.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-79", "Mind change (type)", OsdTheme::Type)
        .feature(model_signals::MIND_CHANGE)
        .moment(Moment::Lookahead)
}
