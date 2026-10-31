//! MO-80 stroke-order writing on the phosphor look: the pen writes each new
//! word in green wet ink with a pale nib light, and the beam only rewrites
//! the letters a revision changed.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: phosphor", OsdTheme::Phosphor)
        .feature("stroke-write")
        .feature("beam-write")
        .moments(&[Moment::Dictation, Moment::RevisionBurst])
}
