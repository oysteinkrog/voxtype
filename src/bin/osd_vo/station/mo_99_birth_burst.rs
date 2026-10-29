//! MO-99 birth burst on the swiss look (ASR-06): a ring opening from each new word, wider when words come together.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-99", "Birth burst", OsdTheme::Swiss)
        .feature(model_signals::BIRTH_BURST)
        .moment(Moment::Lookahead)
}
