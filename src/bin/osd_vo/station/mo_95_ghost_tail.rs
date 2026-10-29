//! MO-95 ghost tail on the swiss look (ASR-05): the lookahead's words follow the live text at low ink and firm up in place when they commit.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-95", "Ghost tail", OsdTheme::Swiss)
        .feature(model_signals::GHOST_TAIL)
        .moment(Moment::Lookahead)
}
