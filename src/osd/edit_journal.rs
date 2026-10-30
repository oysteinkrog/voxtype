//! The text of a discarded keyboard-edit dictation, kept for a short while.
//!
//! A hesitant double tap of the cancel key throws away a dictation that may
//! hold typed edits. Discarding one that was edited keeps its last text in
//! `discarded.json` in the runtime folder for [`KEEP`]; `voxtype record
//! restore` copies it to the clipboard. The file is removed when it is
//! older than that.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long a discarded text is kept.
pub const KEEP: Duration = Duration::from_secs(60);

/// `discarded.json` in the runtime folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discarded {
    /// Milliseconds since the Unix epoch when the dictation was discarded.
    pub at_ms: u64,
    pub text: String,
}

impl Discarded {
    fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.at_ms) >= KEEP.as_millis() as u64
    }
}

pub fn discarded_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("discarded.json")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Keep `text` at `path`, readable by the user only. The folder is made
/// private when it has to be created.
pub fn keep_in(path: &Path, text: &str, at_ms: u64) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent().filter(|d| !d.exists()) {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let json = serde_json::to_string(&Discarded {
        at_ms,
        text: text.to_string(),
    })?;
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(json.as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// The kept text, when there is one younger than [`KEEP`]. An older one is
/// removed.
pub fn restore_from(path: &Path, now_ms: u64) -> Option<String> {
    let kept: Discarded = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if kept.expired(now_ms) {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(kept.text)
}

/// Remove the text kept at `at_ms` once it is older than [`KEEP`]. A text
/// kept later by another discard is left alone.
pub fn expire_in(path: &Path, at_ms: u64, now_ms: u64) {
    let kept = std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str::<Discarded>(&json).ok());
    if kept.is_some_and(|k| k.at_ms == at_ms && k.expired(now_ms)) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn a_discarded_text_is_kept_for_a_minute() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("discarded.json");
        keep_in(&path, "fix this\nline", 1_000).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            restore_from(&path, 1_000).as_deref(),
            Some("fix this\nline")
        );
        assert_eq!(
            restore_from(&path, 60_999).as_deref(),
            Some("fix this\nline")
        );
        // Restoring leaves it for another try.
        assert!(path.exists());
        assert_eq!(restore_from(&path, 61_000), None);
        assert!(!path.exists());
    }

    #[test]
    fn expiry_removes_only_its_own_text() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("discarded.json");
        keep_in(&path, "first", 1_000).unwrap();
        // Too early: kept.
        expire_in(&path, 1_000, 30_000);
        assert!(path.exists());
        // A second discard replaces it; the first one's expiry leaves it.
        keep_in(&path, "second", 20_000).unwrap();
        expire_in(&path, 1_000, 61_000);
        assert_eq!(restore_from(&path, 61_000).as_deref(), Some("second"));
        expire_in(&path, 20_000, 80_000);
        assert!(!path.exists());
    }

    #[test]
    fn the_folder_is_made_private() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("voxtype").join("discarded.json");
        keep_in(&path, "x", 0).unwrap();
        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn nothing_kept_restores_nothing() {
        let dir = TempDir::new().unwrap();
        assert_eq!(restore_from(&dir.path().join("discarded.json"), 0), None);
    }
}
