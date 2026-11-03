//! Texts of keyboard-edit dictations that did not reach their window, kept
//! for a short while for `voxtype record restore`.
//!
//! - A discard: a hesitant double tap of the cancel key throws away a
//!   dictation that may hold typed edits. Discarding one that was edited
//!   keeps its last text in `discarded.json` in the runtime folder for
//!   [`KEEP`].
//! - A crash: while editing, the popup writes its text to `edit-buffer.json`
//!   on every change (one small tmpfs write). Delivery and a new dictation
//!   remove it, so a buffer still there when the daemon starts was never
//!   delivered: the daemon died, or was stopped, during the edit.
//!   [`rescue_in`] moves its text to `edit-journal.json`, offered for
//!   [`CRASH_KEEP`]. It is never pasted by itself.
//!
//! Both files are removed once they are older than their window. Restoring
//! a crashed text removes its journal. Nothing here is logged or retained
//! anywhere else.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long a discarded text is kept.
pub const KEEP: Duration = Duration::from_secs(60);

/// How long the text of a dictation left by a crash is offered back.
pub const CRASH_KEEP: Duration = Duration::from_secs(600);

/// `discarded.json` and `edit-journal.json` in the runtime folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discarded {
    /// Milliseconds since the Unix epoch when the dictation was discarded,
    /// or, for a crash, when its text last changed.
    pub at_ms: u64,
    pub text: String,
}

impl Discarded {
    fn expired(&self, now_ms: u64, keep: Duration) -> bool {
        now_ms.saturating_sub(self.at_ms) >= keep.as_millis() as u64
    }
}

pub fn discarded_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("discarded.json")
}

/// The file a dictation left by a crash is kept in.
pub const JOURNAL_FILE: &str = "edit-journal.json";

pub fn journal_path() -> PathBuf {
    crate::config::Config::runtime_dir().join(JOURNAL_FILE)
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

/// The text kept at `path` when it is younger than `keep`. An older one is
/// removed.
fn read_kept(path: &Path, now_ms: u64, keep: Duration) -> Option<Discarded> {
    let kept: Discarded = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if kept.expired(now_ms, keep) {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(kept)
}

/// The kept discarded text, when there is one younger than [`KEEP`]. An
/// older one is removed.
pub fn restore_from(path: &Path, now_ms: u64) -> Option<String> {
    read_kept(path, now_ms, KEEP).map(|k| k.text)
}

/// Remove the text kept at `at_ms` once it is older than [`KEEP`]. A text
/// kept later by another discard is left alone.
pub fn expire_in(path: &Path, at_ms: u64, now_ms: u64) {
    expire_kept(path, at_ms, now_ms, KEEP);
}

/// [`expire_in`] for the crash journal and [`CRASH_KEEP`].
pub fn expire_journal_in(path: &Path, at_ms: u64, now_ms: u64) {
    expire_kept(path, at_ms, now_ms, CRASH_KEEP);
}

fn expire_kept(path: &Path, at_ms: u64, now_ms: u64, keep: Duration) {
    let kept = std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str::<Discarded>(&json).ok());
    if kept.is_some_and(|k| k.at_ms == at_ms && k.expired(now_ms, keep)) {
        let _ = std::fs::remove_file(path);
    }
}

/// Last change time of `path`, in Unix milliseconds.
fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// On daemon start: turn an edit the last daemon never delivered into the
/// crash journal in `dir` (the runtime folder). The text is the popup's
/// last buffer, plus words the daemon froze after the popup last looked
/// (from `preview.json`). With no buffer, it is the frozen text of a
/// dictation paused for an edit. Returns the journal's time when one was
/// written. The edit files it read are removed, so the same text is not
/// offered twice.
pub fn rescue_in(dir: &Path, now_ms: u64) -> Option<u64> {
    use crate::osd::edit_ipc::{read_buffer_from, with_unseen, EditBlock};
    let buffer_file = dir.join("edit-buffer.json");
    let preview_file = dir.join("preview.json");
    let buffer = read_buffer_from(&buffer_file);
    let edit: Option<EditBlock> = std::fs::read(&preview_file)
        .ok()
        .and_then(|json| serde_json::from_slice::<crate::osd::live::LiveDoc>(&json).ok())
        .and_then(|doc| doc.edit);
    let found = match (&buffer, &edit) {
        (Some(b), Some(e)) if e.session == b.session => Some((
            with_unseen(&b.text, b.base.as_deref(), &e.frozen),
            modified_ms(&buffer_file).max(modified_ms(&preview_file)),
        )),
        (Some(b), _) => Some((b.text.clone(), modified_ms(&buffer_file))),
        (None, Some(e)) if e.phase != "pending" => {
            Some((e.frozen.clone(), modified_ms(&preview_file)))
        }
        _ => None,
    };
    crate::osd::edit_ipc::clear_in(dir);
    let (text, at_ms) = found?;
    let at_ms = at_ms.unwrap_or(now_ms);
    if text.trim().is_empty() || now_ms.saturating_sub(at_ms) >= CRASH_KEEP.as_millis() as u64 {
        return None;
    }
    keep_in(&dir.join(JOURNAL_FILE), &text, at_ms).ok()?;
    Some(at_ms)
}

/// A text `voxtype record restore` can give back.
#[derive(Clone, Debug, PartialEq)]
pub struct Restorable {
    pub text: String,
    /// From the crash journal, which restoring removes. A discarded text
    /// stays for another try until its minute is up.
    pub crashed: bool,
}

/// The newest text to restore: a discarded one younger than [`KEEP`] or a
/// crashed one younger than [`CRASH_KEEP`]. Expired files are removed.
pub fn restorable(discarded: &Path, journal: &Path, now_ms: u64) -> Option<Restorable> {
    let gone = read_kept(discarded, now_ms, KEEP);
    let crashed = read_kept(journal, now_ms, CRASH_KEEP);
    match (gone, crashed) {
        (Some(g), Some(c)) if c.at_ms > g.at_ms => Some(Restorable {
            text: c.text,
            crashed: true,
        }),
        (Some(g), _) => Some(Restorable {
            text: g.text,
            crashed: false,
        }),
        (None, Some(c)) => Some(Restorable {
            text: c.text,
            crashed: true,
        }),
        (None, None) => None,
    }
}

/// After a successful restore: a crashed text's journal is removed.
pub fn restored(r: &Restorable, journal: &Path) {
    if r.crashed {
        let _ = std::fs::remove_file(journal);
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

    use crate::osd::edit_ipc::{write_buffer_to, EditBuffer};

    /// What the popup writes while editing.
    fn popup_buffer(dir: &Path, text: &str) {
        let buffer = EditBuffer {
            session: 4,
            rev: 9,
            text: text.into(),
            sel: (0, 0),
            pause: 1,
            base: None,
        };
        write_buffer_to(&dir.join("edit-buffer.json"), &buffer).unwrap();
    }

    fn private_dir() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("voxtype");
        std::fs::DirBuilder::new()
            .recursive(true)
            .create(&dir)
            .unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        (root, dir)
    }

    #[test]
    fn an_edit_left_by_a_crash_is_restored_byte_for_byte() {
        let (_root, dir) = private_dir();
        let text = "Første linje, rettet\n  med\tmellomrom  og «sitat» \u{1F600}\r\n";
        // The popup wrote its buffer, then the daemon died: nothing delivered it.
        popup_buffer(&dir, text);
        let now = now_ms();
        // The daemon starts again.
        let at = rescue_in(&dir, now).expect("a journal is written");
        assert!(now.saturating_sub(at) < 5_000);
        let journal = dir.join(JOURNAL_FILE);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&journal), 0o600);
        assert_eq!(mode(&dir), 0o700);
        assert!(!dir.join("edit-buffer.json").exists(), "offered once");
        // A second start offers nothing new and keeps the journal.
        assert_eq!(rescue_in(&dir, now), None);
        let r = restorable(&dir.join("discarded.json"), &journal, now).unwrap();
        assert_eq!(r.text.as_bytes(), text.as_bytes());
        assert!(r.crashed);
        restored(&r, &journal);
        assert!(!journal.exists(), "restoring removes the journal");
        assert_eq!(restorable(&dir.join("discarded.json"), &journal, now), None);
    }

    #[test]
    fn the_buffer_file_is_private() {
        let (_root, dir) = private_dir();
        popup_buffer(&dir, "x");
        let mode = std::fs::metadata(dir.join("edit-buffer.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_delivered_dictation_leaves_no_journal() {
        let (_root, dir) = private_dir();
        popup_buffer(&dir, "sent text");
        // Delivery removes the edit files.
        crate::osd::edit_ipc::clear_in(&dir);
        assert_eq!(rescue_in(&dir, now_ms()), None);
        assert!(!dir.join(JOURNAL_FILE).exists());
    }

    #[test]
    fn words_frozen_after_the_popup_looked_are_kept() {
        let (_root, dir) = private_dir();
        let buffer = EditBuffer {
            session: 2,
            rev: 3,
            text: "Hei, fikset".into(),
            sel: (0, 0),
            pause: 1,
            base: Some("Hei fiks".into()),
        };
        write_buffer_to(&dir.join("edit-buffer.json"), &buffer).unwrap();
        let doc = crate::osd::live::LiveDoc {
            edit: Some(crate::osd::edit_ipc::EditBlock {
                session: 2,
                phase: "editing".into(),
                frozen: "Hei fiks og mer".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        std::fs::write(dir.join("preview.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
        rescue_in(&dir, now_ms()).unwrap();
        let r = restorable(
            &dir.join("discarded.json"),
            &dir.join(JOURNAL_FILE),
            now_ms(),
        )
        .unwrap();
        assert!(r.text.starts_with("Hei, fikset"), "{}", r.text);
        assert!(r.text.ends_with("og mer"), "{}", r.text);
    }

    #[test]
    fn a_crash_older_than_ten_minutes_is_not_offered() {
        let (_root, dir) = private_dir();
        popup_buffer(&dir, "old text");
        let later = now_ms() + CRASH_KEEP.as_millis() as u64 + 1_000;
        assert_eq!(rescue_in(&dir, later), None);
        assert!(!dir.join(JOURNAL_FILE).exists());
        assert!(!dir.join("edit-buffer.json").exists());

        // A journal past its window is removed when read, and by its expiry.
        let journal = dir.join(JOURNAL_FILE);
        keep_in(&journal, "kept", 1_000).unwrap();
        expire_journal_in(&journal, 1_000, 500_000);
        assert!(journal.exists());
        expire_journal_in(&journal, 1_000, 601_000);
        assert!(!journal.exists());
        keep_in(&journal, "kept", 1_000).unwrap();
        assert_eq!(
            restorable(&dir.join("discarded.json"), &journal, 601_000),
            None
        );
        assert!(!journal.exists());
    }

    #[test]
    fn a_discarded_text_is_gone_after_its_minute_and_the_newest_wins() {
        let (_root, dir) = private_dir();
        let (gone, journal) = (dir.join("discarded.json"), dir.join(JOURNAL_FILE));
        keep_in(&journal, "crashed", 1_000).unwrap();
        keep_in(&gone, "discarded", 5_000).unwrap();
        let r = restorable(&gone, &journal, 6_000).unwrap();
        assert_eq!((r.text.as_str(), r.crashed), ("discarded", false));
        restored(&r, &journal);
        assert!(gone.exists() && journal.exists());
        // The discard's 60 s run out: its file is gone, the crash is next.
        expire_in(&gone, 5_000, 65_000);
        assert!(!gone.exists());
        let r = restorable(&gone, &journal, 65_000).unwrap();
        assert_eq!((r.text.as_str(), r.crashed), ("crashed", true));
    }

    #[test]
    fn nothing_paused_rescues_nothing() {
        let (_root, dir) = private_dir();
        assert_eq!(rescue_in(&dir, now_ms()), None);
        // A plain dictation's preview has no edit block.
        let doc = crate::osd::live::LiveDoc {
            text: "words".into(),
            ..Default::default()
        };
        std::fs::write(dir.join("preview.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(rescue_in(&dir, now_ms()), None);
    }

    #[test]
    fn a_process_that_exits_mid_edit_is_offered_back() {
        const TEXT: &str = "Rettet tekst\nmed æ, ø og å \u{2014} ferdig ";
        if let Ok(dir) = std::env::var("VOXTYPE_TEST_JOURNAL_CHILD") {
            // The child: the popup's last write, then the process ends with
            // nothing delivered.
            popup_buffer(Path::new(&dir), TEXT);
            std::process::exit(3);
        }
        let (_root, dir) = private_dir();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "osd::edit_journal::tests::a_process_that_exits_mid_edit_is_offered_back",
                "--test-threads=1",
            ])
            .env("VOXTYPE_TEST_JOURNAL_CHILD", &dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(3), "the child stopped mid-edit");
        // The next start offers it back, byte for byte.
        let now = now_ms();
        rescue_in(&dir, now).expect("a journal is written");
        let r = restorable(&dir.join("discarded.json"), &dir.join(JOURNAL_FILE), now).unwrap();
        assert_eq!(r.text.as_bytes(), TEXT.as_bytes());
    }
}
