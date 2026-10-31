//! MO-87 on the chrome look: the old letters of a revised word flow into the new ones.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-87", "Liquid morph (chrome)", OsdTheme::Chrome)
        .feature("liquid-morph")
        .moment(Moment::RevisionBurst)
}
