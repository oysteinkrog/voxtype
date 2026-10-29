//! MO-94 graded doubt on the type look (ASR-06): an underline whose weight follows the word's entropy.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-94", "Graded doubt (type)", OsdTheme::Type)
        .feature(model_signals::GRADED_DOUBT)
        .moment(Moment::Lookahead)
}
