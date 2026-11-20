//! The last dictation's text, for `voxtype history copy` when history is off.
//!
//! The daemon writes `last-dictation.json` in the runtime folder, mode 0600,
//! for every dictation whose final text was made: delivered or not, whatever
//! the output driver said. A paste the target window never received, or one
//! `restore_clipboard` took back off the clipboard, still leaves the text
//! here. Unlike `result.json`, which the popup reads and the next recording
//! removes, this file stays until the next text replaces it, and is gone at
//! logout with the runtime folder.
//!
//! ```json
//! {"text": "Hello there.", "time": "2026-11-20T11:13:02+01:00",
//!  "delivered": true, "method": "paste", "error": null}
//! ```

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One dictation's final text and how its delivery ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastDictation {
    pub text: String,
    /// When the delivery ended, RFC 3339 local time.
    pub time: String,
    /// The output driver reported success. Not a proof the text landed.
    pub delivered: bool,
    /// The driver that took the text, such as "paste" or "wtype".
    pub method: Option<String>,
    /// Why the delivery failed, when it did.
    pub error: Option<String>,
}

impl LastDictation {
    /// A dictation ending now.
    pub fn now(text: &str, delivered: Result<&str, String>) -> Self {
        let (delivered, method, error) = match delivered {
            Ok(m) => (true, Some(m.to_string()), None),
            Err(e) => (false, None, Some(e)),
        };
        Self {
            text: text.to_string(),
            time: chrono::Local::now().to_rfc3339(),
            delivered,
            method,
            error,
        }
    }
}

/// `last-dictation.json` in the runtime folder.
pub fn path() -> PathBuf {
    crate::config::Config::runtime_dir().join("last-dictation.json")
}

/// Keep `last` in the runtime folder. Best effort: a failure is logged, the
/// dictation goes on.
pub fn write(last: &LastDictation) {
    if let Err(e) = write_at(&path(), last) {
        tracing::warn!("Could not keep the last dictation for `voxtype history copy`: {e}");
    }
}

/// [`write`] to `path`.
pub fn write_at(path: &Path, last: &LastDictation) -> std::io::Result<()> {
    let json = serde_json::to_vec(last).map_err(std::io::Error::other)?;
    write_private(path, &json)
}

/// The last kept dictation, if there is one and it parses.
pub fn read() -> Option<LastDictation> {
    read_at(&path())
}

/// [`read`] from `path`.
pub fn read_at(path: &Path) -> Option<LastDictation> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Write `bytes` to `path` readable by the user alone (0600 on Unix), via a
/// temporary file and a rename, so a reader never sees half of it. Creates
/// the folder (0700 on Unix) when it is missing.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.exists() {
            let mut b = std::fs::DirBuilder::new();
            b.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b.create(dir)?;
        }
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&tmp)?;
    f.write_all(bytes)?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_kept_dictation_reads_back() {
        let dir = temp();
        let p = dir.path().join("last-dictation.json");
        let last = LastDictation::now("The quick brown fox.", Ok("paste"));
        write_at(&p, &last).unwrap();
        assert_eq!(read_at(&p), Some(last));
    }

    #[test]
    fn a_failed_delivery_keeps_its_text() {
        let dir = temp();
        let p = dir.path().join("last-dictation.json");
        write_at(
            &p,
            &LastDictation::now("Jumps over the lazy dog.", Err("no driver".into())),
        )
        .unwrap();
        let back = read_at(&p).unwrap();
        assert_eq!(back.text, "Jumps over the lazy dog.");
        assert!(!back.delivered);
        assert_eq!(back.error.as_deref(), Some("no driver"));
    }

    #[test]
    fn the_newest_text_replaces_the_last() {
        let dir = temp();
        let p = dir.path().join("last-dictation.json");
        write_at(&p, &LastDictation::now("First made-up line.", Ok("wtype"))).unwrap();
        write_at(&p, &LastDictation::now("Second made-up line.", Ok("wtype"))).unwrap();
        assert_eq!(read_at(&p).unwrap().text, "Second made-up line.");
    }

    #[cfg(unix)]
    #[test]
    fn only_the_user_can_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp();
        let p = dir.path().join("sub").join("last-dictation.json");
        write_at(&p, &LastDictation::now("A made-up secret.", Ok("paste"))).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file mode {mode:o}");
        let dmode = std::fs::metadata(p.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700, "folder mode {dmode:o}");
    }

    #[test]
    fn nothing_kept_reads_as_none() {
        let dir = temp();
        assert_eq!(read_at(&dir.path().join("last-dictation.json")), None);
    }
}
