//! MO-100 sound strip on the swiss look (ASR-10): the speech sounds the layer 8 phone probe hears, under the text and ahead of the words.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-100", "Sound strip", OsdTheme::Swiss)
        .feature(model_signals::SOUND_STRIP)
        .moment(Moment::Lookahead)
}
