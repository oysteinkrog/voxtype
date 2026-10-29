//! MO-67 grid of onsets on synthwave: each onset sends one grid row from
//! the horizon, so the road's spacing is the speech rhythm and a pause is
//! empty road.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-67", "Grid of onsets", OsdTheme::Synthwave)
        .feature(synthwave::ONSET_GRID)
        .moments(&[Moment::Pause, Moment::LongSilence])
}
