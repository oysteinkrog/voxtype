//! MO-96 listening floor on the showcase look (ASR-05): a ribbon under the text from log10(1 - P(blank)) of each 80 ms frame.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-96", "Listening floor (showcase)", OsdTheme::Showcase)
        .feature(model_signals::FLOOR)
        .moment(Moment::Lookahead)
}
