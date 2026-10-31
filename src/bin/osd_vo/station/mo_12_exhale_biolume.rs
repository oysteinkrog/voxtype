//! MO-12 on the biolume look: the waveform flattens in a long silence and springs back on the next word.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-12", "Exhale and snap back (biolume)", OsdTheme::Biolume)
        .feature("exhale")
        .moment(Moment::LongSilence)
}
