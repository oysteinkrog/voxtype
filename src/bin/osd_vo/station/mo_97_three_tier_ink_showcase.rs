//! MO-97 three-tier ink on the showcase look (ASR-05): committed words at full ink, pending tail words at 70 %, edge guesses at 30 %.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-97", "Three-tier ink (showcase)", OsdTheme::Showcase)
        .feature(model_signals::THREE_TIER)
        .moment(Moment::Lookahead)
}
