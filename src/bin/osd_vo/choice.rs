//! MO-75: each look's own pick of the shared motions. With `look-choice` on, a
//! look takes the cancel (MO-50 rewind or MO-51 unthread) and the overflow
//! (MO-33 fold-under or MO-32 margin mark) that suit its material, as if
//! those features had been asked for by name. The revision (MO-20 and its
//! variants) is already the look's own: `revision.rs` picks escrow, hinge and
//! address per look, and cyberpunk takes the palimpsest. The hand-off (MO-42)
//! has one form, drawn in each look's colours, so there is nothing to pick
//! there.
//!
//! A pick only shows in a look that draws that motion; the others ignore it.
//! Reduced motion still wins: every motion has its still form.

use super::*;

/// Switches on each look's picks.
pub(crate) const LOOK_CHOICE: &str = "look-choice";

/// The motion features `theme` picks.
pub(crate) fn picks(theme: OsdTheme) -> &'static [&'static str] {
    use OsdTheme::*;
    match theme {
        // Instruments and screens: the words wind back into the caret, and
        // the top line folds under like a page in a machine.
        Cyberpunk | Mecha | Tron | Amber | Scope => &["rewind", "fold-under"],
        Synthwave | Jarvis | Palette | Mission | Radial => &["fold-under"],
        // Calm text: a thread pulls back under the words, and the newest
        // line's bar moves when the text wraps.
        Swissfi => &["unthread", "margin-mark"],
        Karaoke => &["rewind", "margin-mark"],
        Captions | Island => &["margin-mark"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_look_picks_one_cancel_and_one_overflow_at_most() {
        for theme in [
            OsdTheme::Cyberpunk,
            OsdTheme::Synthwave,
            OsdTheme::Mecha,
            OsdTheme::Tron,
            OsdTheme::Jarvis,
            OsdTheme::Palette,
            OsdTheme::Mission,
            OsdTheme::Amber,
            OsdTheme::Radial,
            OsdTheme::Captions,
            OsdTheme::Karaoke,
            OsdTheme::Scope,
            OsdTheme::Island,
            OsdTheme::Swissfi,
        ] {
            let p = picks(theme);
            let n = |names: &[&str]| p.iter().filter(|f| names.contains(f)).count();
            assert!(n(&["rewind", "unthread"]) <= 1, "{theme:?}");
            assert_eq!(n(&["fold-under", "margin-mark"]), 1, "{theme:?}");
        }
    }
}
