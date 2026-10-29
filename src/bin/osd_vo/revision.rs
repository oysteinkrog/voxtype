//! What a revision does around the changed word (MO-21 to MO-23), per look: width escrow
//! in the calm text looks, the correction hinge and the revision address in the
//! instrument looks. Any look takes them with `--motion width-escrow,correction-hinge,
//! revision-address`. The palimpsest itself (MO-20) is the framework default everywhere.
//!
//! `liquid-revise` (EG-32): a revised word flows into its correction as liquid
//! (`Revise::Liquid`), in the word's own colour or material, in these looks.

use super::*;
use vo::scene::flow::revisions::RevisionMotion;
use vo::scene::flow::Revise;

pub(crate) const LIQUID: &str = "liquid-revise";

/// How a revised word changes its letters in `look`: the liquid with
/// `liquid-revise`, else the framework default, the palimpsest.
pub(crate) fn revise(look: &Look) -> Revise {
    if look.on(LIQUID) {
        // Every renderer the popup makes runs `vo_3d::install`, which draws
        // the liquid; say so now, since the scene may revise a word before
        // the first renderer exists (an `--envelope` run, a CPU first frame).
        vo::scene::flow::liquid::install_painter();
        Revise::Liquid
    } else {
        Revise::Palimpsest
    }
}

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
