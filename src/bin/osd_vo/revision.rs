//! What a revision does around the changed word (MO-21 to MO-23), per look: width escrow
//! in the calm text looks, the correction hinge and the revision address in the
//! instrument looks. Any look takes them with `--motion width-escrow,correction-hinge,
//! revision-address`. The palimpsest itself (MO-20) is the framework default everywhere.

use super::*;
use vo::scene::flow::revisions::RevisionMotion;

/// The revision motion `look` asks for.
pub(crate) fn motion(look: &Look) -> RevisionMotion {
    let t = look.theme;
    RevisionMotion {
        escrow: look.on("width-escrow")
            || matches!(
                t,
                OsdTheme::Default
                    | OsdTheme::Swiss
                    | OsdTheme::Type
                    | OsdTheme::Captions
                    | OsdTheme::Island
            ),
        hinge: look.on("correction-hinge")
            || matches!(
                t,
                OsdTheme::Mecha | OsdTheme::Mission | OsdTheme::Swissfi | OsdTheme::Jarvis
            ),
        address: look.on("revision-address")
            || matches!(
                t,
                OsdTheme::Captions | OsdTheme::Mission | OsdTheme::Swissfi | OsdTheme::Jarvis
            ),
    }
}
