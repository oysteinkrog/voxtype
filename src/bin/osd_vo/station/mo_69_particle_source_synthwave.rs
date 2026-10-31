//! MO-69 on the synthwave look: each word forms from particles that leave the waveform.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-69", "Particle source (synthwave)", OsdTheme::Synthwave)
        .feature("particle-source")
        .moment(Moment::Dictation)
}
