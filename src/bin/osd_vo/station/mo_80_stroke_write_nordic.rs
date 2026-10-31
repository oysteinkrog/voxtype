//! MO-80 stroke writing on Norwegian, accented, CJK and emoji words: æ, ø
//! and å are written by their own centre lines, with the ring and the slash
//! held to the end of the word, and a glyph no pen line stands for (an
//! emoji, a missing glyph) is swept in.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-80", "Stroke writing: nordic", OsdTheme::Type)
        .feature("stroke-write")
        .moment(Moment::Nordic)
}
