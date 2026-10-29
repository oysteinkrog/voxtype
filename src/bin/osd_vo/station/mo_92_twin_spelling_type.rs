//! MO-92 twin spelling on the type look (ASR-06): the runner-up the model nearly wrote, fading above the word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-92", "Twin spelling (type)", OsdTheme::Type)
        .feature(model_signals::TWIN_SPELLING)
        .moment(Moment::Lookahead)
}
