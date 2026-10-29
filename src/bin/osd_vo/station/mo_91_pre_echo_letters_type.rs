//! MO-91 pre-echo letters on the type look (ASR-06): the first letters of the model's strong next guess, one frame early.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-91", "Pre-echo letters (type)", OsdTheme::Type)
        .feature(model_signals::PRE_ECHO_LETTERS)
        .moment(Moment::Lookahead)
}
