//! MO-98 commit cursor on the showcase look (ASR-06): a hairline where the committed text ends and one where the model has heard to.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-98", "Commit cursor (showcase)", OsdTheme::Showcase)
        .feature(model_signals::COMMIT_CURSOR)
        .moment(Moment::Lookahead)
}
