//! Switching the vo popup's look from the keyboard.
//!
//! The popup never has keyboard focus, so the daemon's evdev listener reads
//! the chord (Left Ctrl and Right Ctrl held, then Up or Down) and writes a
//! step to `osd-look` in the runtime folder. An open popup steps its look
//! when that file changes and saves the new look to `osd-look` in
//! voxtype's state folder, where later popups read it. A closed popup
//! ignores steps.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::config::OsdTheme;

/// The step file's name in the runtime folder.
pub const FILE: &str = "osd-look";

/// One press of the chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// When it was pressed, in nanoseconds since the epoch, so two presses
    /// in the same direction still differ.
    pub stamp: u64,
    /// Down: the next look. Up: the previous one.
    pub forward: bool,
}

impl Step {
    fn parse(s: &str) -> Option<Step> {
        let (stamp, dir) = s.trim().split_once(' ')?;
        let forward = match dir {
            "next" => true,
            "prev" => false,
            _ => return None,
        };
        Some(Step {
            stamp: stamp.parse().ok()?,
            forward,
        })
    }
}

/// `osd-look` in the runtime folder.
pub fn path() -> PathBuf {
    crate::config::Config::runtime_dir().join(FILE)
}

/// Record a press of the chord. Written to a temporary name and renamed, so
/// a reader never sees half of it.
pub fn write(forward: bool) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let path = path();
    let tmp = path.with_extension("tmp");
    let text = format!("{stamp} {}", if forward { "next" } else { "prev" });
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The last press of the chord, if any.
pub fn read() -> Option<Step> {
    Step::parse(&std::fs::read_to_string(path()).ok()?)
}

/// `$XDG_STATE_HOME/voxtype/osd-look`: the look picked with the chord.
fn saved_path() -> Option<PathBuf> {
    Some(dirs::state_dir()?.join("voxtype").join(FILE))
}

/// The look picked with the chord. It wins over `[osd] theme` and `--theme`.
pub fn saved() -> Option<OsdTheme> {
    OsdTheme::parse_str(&std::fs::read_to_string(saved_path()?).ok()?)
}

/// Keep `theme` for later popups.
pub fn save(theme: OsdTheme) {
    let Some(path) = saved_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let name = format!("{theme:?}").to_ascii_lowercase();
    if let Err(e) = std::fs::write(&path, name) {
        tracing::warn!("cannot save the popup look to {}: {e}", path.display());
    }
}

/// The look after (or before) `current` in `order`, wrapping round. A look
/// that is not in `order` steps to its first (or last) entry.
pub fn step(order: &[OsdTheme], current: OsdTheme, forward: bool) -> OsdTheme {
    let n = order.len();
    if n == 0 {
        return current;
    }
    match order.iter().position(|&t| t == current) {
        Some(i) if forward => order[(i + 1) % n],
        Some(i) => order[(i + n - 1) % n],
        None if forward => order[0],
        None => order[n - 1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_wrap_both_ways() {
        let order = [OsdTheme::Swiss, OsdTheme::Type, OsdTheme::Mecha];
        assert_eq!(step(&order, OsdTheme::Swiss, true), OsdTheme::Type);
        assert_eq!(step(&order, OsdTheme::Mecha, true), OsdTheme::Swiss);
        assert_eq!(step(&order, OsdTheme::Swiss, false), OsdTheme::Mecha);
        assert_eq!(step(&order, OsdTheme::Type, false), OsdTheme::Swiss);
        // A look outside the order enters it at either end.
        assert_eq!(step(&order, OsdTheme::Amber, true), OsdTheme::Swiss);
        assert_eq!(step(&order, OsdTheme::Amber, false), OsdTheme::Mecha);
    }

    #[test]
    fn step_file_parses() {
        assert_eq!(
            Step::parse("17 next\n"),
            Some(Step {
                stamp: 17,
                forward: true
            })
        );
        assert_eq!(
            Step::parse("18 prev"),
            Some(Step {
                stamp: 18,
                forward: false
            })
        );
        assert_eq!(Step::parse("18 sideways"), None);
        assert_eq!(Step::parse(""), None);
    }

    #[test]
    fn the_chord_visits_all_27_looks_once_and_comes_back() {
        let order = OsdTheme::VO_LOOKS;
        assert_eq!(order.len(), 27);
        let mut seen = Vec::new();
        let mut look = OsdTheme::Swiss;
        for _ in 0..order.len() {
            assert!(!seen.contains(&look), "{look:?} twice");
            seen.push(look);
            look = step(&order, look, true);
        }
        assert_eq!(look, OsdTheme::Swiss);
        assert!(order.iter().all(|t| seen.contains(t)));
    }

    #[test]
    fn saved_names_parse_back() {
        for theme in OsdTheme::VO_LOOKS {
            let name = format!("{theme:?}").to_ascii_lowercase();
            assert_eq!(OsdTheme::parse_str(&name), Some(theme));
        }
    }
}
