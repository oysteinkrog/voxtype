//! MO-21 correction hinge: when one word becomes several or several become one, a thin
//! bracket marks the span and its joints move from the old word breaks to the new ones.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-21", "Correction hinge", OsdTheme::Mission)
        .feature("correction-hinge")
        .moment(Moment::RevisionBurst)
}
