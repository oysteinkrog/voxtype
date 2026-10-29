//! MO-99 birth burst on the showcase look (ASR-06): a ring opening from each new word, wider when words come together.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-99", "Birth burst (showcase)", OsdTheme::Showcase)
        .feature(model_signals::BIRTH_BURST)
        .moment(Moment::Lookahead)
}
