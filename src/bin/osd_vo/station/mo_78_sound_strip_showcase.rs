//! MO-78 sound strip on the showcase look (ASR-10): the speech sounds the layer 8 phone probe hears, after the text and ahead of the words.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-78", "Sound strip (showcase)", OsdTheme::Showcase)
        .feature(model_signals::SOUND_STRIP)
        .moment(Moment::Lookahead)
}
