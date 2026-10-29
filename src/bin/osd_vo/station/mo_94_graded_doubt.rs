//! MO-94 graded doubt on the swiss look (ASR-06): an underline whose weight follows the word's entropy.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-94", "Graded doubt", OsdTheme::Swiss)
        .feature(model_signals::GRADED_DOUBT)
        .moment(Moment::Lookahead)
}
