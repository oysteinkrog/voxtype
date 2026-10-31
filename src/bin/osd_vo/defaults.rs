//! Each look's own motion set (RD-01): the motion features a look turns on
//! when nobody asks for others. The rows follow the redesign plan
//! (`docs/REDESIGN-PLAN.md` in the framework repo, section 3); effects the
//! plan names in plain words are built into the look and need no entry.
//!
//! `[osd] motion`, `--motion` and `VOXTYPE_OSD_VO_MOTION` pick between this
//! set, no set, or names of their own (`OsdMotion`); `resolve` turns that
//! into what a `Look` keeps.

use std::collections::HashSet;
use std::sync::OnceLock;

use voxtype::osd::config::{OsdMotion, OsdTheme};

/// The motion features `theme` turns on by default, as `--motion` names them.
pub(crate) fn defaults(theme: OsdTheme) -> &'static [&'static str] {
    use OsdTheme::*;
    match theme {
        // vo draws `default` as swiss.
        Default | Swiss => &["pause-marks", "pitch-baseline", "audio-ruler", "fold-under", "pre-echo", "exhale"],
        Showcase => &[
            "particle-source",
            "light-letters",
            "variable-breath",
            "depth-stack",
        ],
        Type => &["onset-ink", "confidence-ink", "width-escrow", "margin-bell", "pre-echo", "pause-marks", "exhale"],
        Amber => &["pause-marks", "margin-bell", "onset-ink", "fold-under"],
        Radial => &["ring-reads", "acoustic-receipts", "audio-ruler", "onset-joint"],
        Tron => &["light-line", "stroke-write", "overflow-spine"],
        Phosphor => &["beam-write", "stroke-write", "broken-frame", "confidence-ink"],
        Mission => &[
            "correction-hinge",
            "limit-hairline",
            "audio-ruler",
            "revision-address",
        ],
        Palette => &["correction-hinge", "width-escrow", "loading-word"],
        Dotmatrix => &["dot-sort", "count-cast", "empty-measure", "confidence-ink"],
        Swissfi => &["spoken-stagger", "count-cast", "pre-echo", "pause-marks", "exhale"],
        Scope => &["sentence-trace", "kinetic-lattice", "onset-joint", "rewind"],
        Captions => &["sound-sculpture", "fold-under", "pitch-baseline", "unthread"],
        Karaoke => &["bowstring", "pitch-score", "spoken-tracking", "margin-mark"],
        Island => &["word-bulge", "lean-pace", "exhale"],
        Jarvis => &["ring-reads", "depth-stack", "onset-ink"],
        Particles => &["particle-source", "letter-physics", "rewind", "pre-echo"],
        Synthwave => &["onset-grid", "light-line", "pitch-baseline", "particle-source"],
        Mecha => &["hydraulic-chain", "onset-joint", "look-choice", "broken-frame"],
        Cyberpunk => &["look-choice", "broken-frame", "rewind"],
        // New looks (RD-03) start with no set; each look's FX bead adds its row.
        #[allow(unreachable_patterns)]
        _ => &[],
    }
}

/// Features in every look's set: the lookahead tail's words follow the live
/// text dim, so the newest words show about 0.56 s sooner (LAT-1), and the
/// forming cue shows that a word is coming as soon as you speak (LAT-5).
const EVERY_LOOK: &[&str] = &[super::model_signals::GHOST_TAIL, super::forming::FORMING];

/// Whether `name` is in `theme`'s own set or in every look's.
pub(crate) fn in_set(theme: OsdTheme, name: &str) -> bool {
    defaults(theme).contains(&name) || EVERY_LOOK.contains(&name)
}

/// Names that change how features draw rather than being one: they never
/// replace the look's set.
fn modifier(name: &str) -> bool {
    name == "reduced-motion" || name.starts_with("limit-secs-") || name.starts_with("no-")
}

/// Every motion feature some station exhibit turns on: the names a look
/// knows. Anything else is ignored.
fn known() -> &'static HashSet<&'static str> {
    static KNOWN: OnceLock<HashSet<&'static str>> = OnceLock::new();
    KNOWN.get_or_init(|| {
        super::station::exhibits()
            .iter()
            .flat_map(|e| e.features.iter().copied())
            .filter(|n| !modifier(n))
            .collect()
    })
}

/// What a `Look` keeps for a motion setting: the names asked for, and
/// whether the look's own set (`defaults`) is on as well. A list turns the
/// set off when it has `none`, or a known feature name and no `look`; a list
/// of only `no-` entries, modifiers and unknown names keeps it.
pub(crate) fn resolve(m: &OsdMotion) -> (Vec<String>, bool) {
    match m {
        OsdMotion::Look => (Vec::new(), true),
        OsdMotion::None => (Vec::new(), false),
        OsdMotion::Names(names) => {
            let has = |w: &str| names.iter().any(|n| n == w);
            let plain = names.iter().any(|n| known().contains(n.as_str()));
            let look_set = !has("none") && (has("look") || !plain);
            let names = names
                .iter()
                .filter(|n| *n != "look" && *n != "none")
                .cloned()
                .collect();
            (names, look_set)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn look(theme: OsdTheme, motion: &OsdMotion) -> Look {
        let cfg = OsdConfig {
            theme,
            ..OsdConfig::default()
        };
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut l = Look::new(&cfg, &plasma, String::new(), &fonts);
        l.set_motion(motion);
        l
    }

    fn names(v: &[&str]) -> OsdMotion {
        OsdMotion::Names(v.iter().map(|s| s.to_string()).collect())
    }

    const SWISS_SET: [&str; 4] = ["pause-marks", "pitch-baseline", "audio-ruler", "fold-under"];

    #[test]
    fn swiss_with_no_motion_config_turns_on_its_set() {
        let l = look(OsdTheme::Swiss, &OsdConfig::default().motion);
        for n in SWISS_SET {
            assert!(l.on(n), "{n}");
        }
        assert!(!l.on("onset-ink"));
    }

    #[test]
    fn motion_none_turns_the_set_off() {
        let l = look(OsdTheme::Swiss, &OsdMotion::None);
        assert!(!l.on("fold-under"));
        for n in SWISS_SET {
            assert!(!l.on(n), "{n}");
        }
    }

    #[test]
    fn a_plain_name_replaces_the_set() {
        let m = OsdMotion::layered(&["onset-ink".to_string()], &OsdMotion::Look);
        let l = look(OsdTheme::Swiss, &m);
        assert!(l.on("onset-ink"));
        for n in SWISS_SET {
            assert!(!l.on(n), "{n}");
        }
    }

    #[test]
    fn cli_none_beats_a_config_list() {
        let m = OsdMotion::layered(&["none".to_string()], &names(&["no-fold-under"]));
        let l = look(OsdTheme::Swiss, &m);
        for n in SWISS_SET {
            assert!(!l.on(n), "{n}");
        }
    }

    #[test]
    fn a_no_entry_takes_one_name_out_of_the_set() {
        let l = look(OsdTheme::Swiss, &names(&["no-fold-under"]));
        assert!(!l.on("fold-under"));
        for n in ["pause-marks", "pitch-baseline", "audio-ruler"] {
            assert!(l.on(n), "{n}");
        }
    }

    #[test]
    fn look_in_a_list_adds_to_the_set() {
        let l = look(OsdTheme::Swiss, &names(&["look", "onset-ink"]));
        assert!(l.on("onset-ink"));
        for n in SWISS_SET {
            assert!(l.on(n), "{n}");
        }
    }

    #[test]
    fn unknown_names_and_modifiers_keep_the_set() {
        let l = look(
            OsdTheme::Swiss,
            &names(&["warp-drive", "reduced-motion", "limit-secs-20"]),
        );
        for n in SWISS_SET {
            assert!(l.on(n), "{n}");
        }
        assert!(l.on("reduced-motion"));
        assert!(!l.on("onset-ink"));
    }

    #[test]
    fn reduced_motion_leaves_the_set_on() {
        let l = look(OsdTheme::Swiss, &names(&["reduced-motion"]));
        assert!(l.on("reduced-motion"));
        for n in SWISS_SET {
            assert!(l.on(n), "{n}");
        }
    }

    #[test]
    fn a_look_switch_answers_the_new_looks_set() {
        let mut l = look(OsdTheme::Swiss, &OsdMotion::Look);
        assert!(l.on("audio-ruler"));
        assert!(!l.on("margin-bell"));
        // The look chord changes the theme in place.
        l.theme = OsdTheme::Amber;
        for n in defaults(OsdTheme::Amber) {
            assert!(l.on(n), "{n}");
        }
        assert!(!l.on("audio-ruler"));
        assert!(!l.on("pitch-baseline"));
    }

    #[test]
    fn look_choice_from_the_set_turns_on_its_picks() {
        let l = look(OsdTheme::Mecha, &OsdMotion::Look);
        for n in super::super::choice::picks(OsdTheme::Mecha) {
            assert!(l.on(n), "{n}");
        }
        // A `no-` entry also beats a pick.
        let l = look(OsdTheme::Mecha, &names(&["no-rewind"]));
        assert!(!l.on("rewind"));
        assert!(l.on("fold-under"));
    }

    #[test]
    fn a_station_exhibit_keeps_only_its_own_features() {
        // `Look::new` starts with no set; the station only writes features.
        for e in super::super::station::exhibits() {
            let cfg = OsdConfig {
                theme: e.theme,
                ..OsdConfig::default()
            };
            let plasma = PlasmaTheme::default();
            let fonts = load_fonts(&plasma).unwrap();
            let mut l = Look::new(&cfg, &plasma, String::new(), &fonts);
            l.features = e.features.iter().map(|f| f.to_string()).collect();
            let picks: &[&str] = if e.features.contains(&"look-choice") {
                super::super::choice::picks(e.theme)
            } else {
                &[]
            };
            for n in defaults(e.theme) {
                let want = e.features.contains(n) || picks.contains(n);
                assert_eq!(l.on(n), want, "{} {n}", e.id);
            }
        }
    }

    #[test]
    fn every_default_name_is_a_station_exhibits_feature() {
        for t in OsdTheme::VO_LOOKS {
            for n in defaults(t) {
                assert!(known().contains(n), "{t:?}: {n} has no station exhibit");
            }
        }
    }

    #[test]
    fn every_vo_look_has_a_set() {
        // The new looks' slots (RD-03) get their sets from their effects
        // beads; each one leaves this list when its row lands.
        const SLOTS: [OsdTheme; 7] = [
            OsdTheme::Orb,
            OsdTheme::Chrome,
            OsdTheme::Glass,
            OsdTheme::Biolume,
            OsdTheme::Aurora,
            OsdTheme::Hologram,
            OsdTheme::River,
        ];
        for t in OsdTheme::VO_LOOKS {
            assert!(SLOTS.contains(&t) || !defaults(t).is_empty(), "{t:?}");
        }
    }
}
