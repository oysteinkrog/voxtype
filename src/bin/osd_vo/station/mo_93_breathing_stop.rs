//! MO-93 breathing full stop on the swiss look (ASR-06): the full stop the lookahead expects, at its probability's ink.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-93", "Breathing full stop", OsdTheme::Swiss)
        .feature(model_signals::BREATHING_STOP)
        .moment(Moment::Lookahead)
}
