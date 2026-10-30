//! The file link between the daemon and the popup for keyboard editing.
//!
//! All files live in the runtime dir (`$XDG_RUNTIME_DIR/voxtype`, tmpfs,
//! mode 0700):
//!
//! - `preview.json` gains an `edit` block ([`EditBlock`]), written by the
//!   daemon when the edit phase or the frozen text changes, never per key.
//! - `edit-buffer.json` ([`EditBuffer`]): the popup's text, one small write
//!   per change while it owns the text.
//! - `edit-cmd`: the popup's commands, one stamped JSON line each
//!   ([`EditCmdLine`]), appended.
//!
//! A command that names a revision is acted on only once the buffer file
//! holds that revision, so the daemon never takes text older than the
//! popup showed.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

/// What the popup needs to edit, as the `edit` block of `preview.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditBlock {
    /// Counts dictations; buffers and commands of another session are
    /// ignored.
    pub session: u64,
    /// `pending`, `settling`, `editing`, `detached` or `releasing`.
    pub phase: String,
    /// The frozen region: processed, normalised, and editable.
    pub frozen: String,
    /// Words still settling after it, not yet editable.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tail: String,
    /// Counts the pauses of this session. A buffer written in the same
    /// pause is the popup's own, so a popup started again reloads it.
    #[serde(default)]
    pub pause: u32,
}

/// `edit-buffer.json`: the popup's text while it owns it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditBuffer {
    pub session: u64,
    /// Goes up with every change.
    pub rev: u64,
    pub text: String,
    /// Selection anchor and focus, byte offsets into `text`.
    pub sel: (usize, usize),
    /// The [`EditBlock::pause`] the popup was editing.
    #[serde(default)]
    pub pause: u32,
}

/// How a finish delivers the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishMode {
    /// Paste only, as a Right Shift double tap.
    Paste,
    /// Paste, then Enter, as a Right Ctrl double tap.
    PasteEnter,
}

/// A command from the popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum EditCmd {
    /// Take the buffer at `rev` and listen again.
    Resume { rev: u64 },
    /// Take the buffer at `rev` and deliver.
    Finish { mode: FinishMode, rev: u64 },
    /// The popup destroyed its focus proxy, so focus is going back.
    ProxyClosed,
    /// The compositor took focus from the popup while editing (a click
    /// elsewhere, Alt+Tab, the lock screen): stay paused, keep the text.
    Detached,
}

/// One line of `edit-cmd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditCmdLine {
    pub session: u64,
    /// When the popup wrote it, Unix milliseconds.
    pub at_ms: u64,
    #[serde(flatten)]
    pub cmd: EditCmd,
}

pub fn buffer_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("edit-buffer.json")
}

pub fn cmd_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("edit-cmd")
}

/// Write the buffer: a temp file renamed over the old one, so a reader
/// never sees half of it.
pub fn write_buffer_to(path: &std::path::Path, buffer: &EditBuffer) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(buffer)?)?;
    std::fs::rename(&tmp, path)
}

pub fn read_buffer_from(path: &std::path::Path) -> Option<EditBuffer> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Append one command line, stamped now.
pub fn append_cmd_to(path: &std::path::Path, session: u64, cmd: EditCmd) -> std::io::Result<()> {
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let mut line = serde_json::to_string(&EditCmdLine {
        session,
        at_ms,
        cmd,
    })?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line.as_bytes())
}

/// Reads the commands appended to `edit-cmd` since the last read. A
/// command that names a revision the buffer file does not hold yet waits
/// for it.
#[derive(Debug, Default)]
pub struct CmdReader {
    /// Bytes of the file already read.
    offset: usize,
    /// Commands read but not acted on yet: the first waits for its buffer
    /// revision, the rest wait behind it.
    queue: std::collections::VecDeque<EditCmd>,
}

/// A command the daemon may act on, with the buffer text it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accepted {
    Resume { text: String },
    Finish { mode: FinishMode, text: String },
    ProxyClosed,
    Detached,
}

impl CmdReader {
    /// Commands of `session` ready to act on, oldest first. `buffer` is the
    /// buffer file as read now.
    pub fn poll(
        &mut self,
        cmd_file: &std::path::Path,
        session: u64,
        buffer: Option<&EditBuffer>,
    ) -> Vec<Accepted> {
        let data = std::fs::read(cmd_file).unwrap_or_default();
        if data.len() < self.offset {
            // The file was removed and started again.
            self.offset = 0;
        }
        // Only whole lines; a line being written waits for the next poll.
        let end = data[self.offset..]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(self.offset, |i| self.offset + i + 1);
        let new = std::str::from_utf8(&data[self.offset..end]).unwrap_or_default();
        self.offset = end;
        self.queue.extend(
            new.lines()
                .filter_map(|l| serde_json::from_str::<EditCmdLine>(l).ok())
                .filter(|l| l.session == session)
                .map(|l| l.cmd),
        );
        // In order: nothing passes a command still waiting.
        let mut ready = Vec::new();
        while let Some(accepted) = self
            .queue
            .front()
            .and_then(|&cmd| accept(cmd, session, buffer))
        {
            self.queue.pop_front();
            ready.push(accepted);
        }
        ready
    }
}

/// `cmd` with the text it names, or None while the buffer file does not
/// hold its revision of this session.
pub fn accept(cmd: EditCmd, session: u64, buffer: Option<&EditBuffer>) -> Option<Accepted> {
    let text = |rev: u64| {
        buffer
            .filter(|b| b.session == session && b.rev == rev)
            .map(|b| b.text.clone())
    };
    match cmd {
        EditCmd::Resume { rev } => text(rev).map(|text| Accepted::Resume { text }),
        EditCmd::Finish { mode, rev } => text(rev).map(|text| Accepted::Finish { mode, text }),
        EditCmd::ProxyClosed => Some(Accepted::ProxyClosed),
        EditCmd::Detached => Some(Accepted::Detached),
    }
}

/// Remove the buffer and command files: after delivery, or when a new
/// dictation starts.
pub fn clear() {
    let _ = std::fs::remove_file(buffer_path());
    let _ = std::fs::remove_file(cmd_path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn buffer(session: u64, rev: u64, text: &str) -> EditBuffer {
        EditBuffer {
            session,
            rev,
            text: text.into(),
            sel: (text.len(), text.len()),
            pause: 1,
        }
    }

    #[test]
    fn the_files_have_the_documented_shape() {
        let block = EditBlock {
            session: 3,
            phase: "pending".into(),
            frozen: "Hello.".into(),
            tail: String::new(),
            pause: 2,
        };
        assert_eq!(
            serde_json::to_string(&block).unwrap(),
            r#"{"session":3,"phase":"pending","frozen":"Hello.","pause":2}"#
        );
        let line = EditCmdLine {
            session: 3,
            at_ms: 9,
            cmd: EditCmd::Finish {
                mode: FinishMode::PasteEnter,
                rev: 4,
            },
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"session":3,"at_ms":9,"cmd":"finish","mode":"paste_enter","rev":4}"#
        );
        let closed: EditCmdLine =
            serde_json::from_str(r#"{"session":1,"at_ms":2,"cmd":"proxy_closed"}"#).unwrap();
        assert_eq!(closed.cmd, EditCmd::ProxyClosed);
        assert_eq!(
            serde_json::to_string(&buffer(1, 2, "hi")).unwrap(),
            r#"{"session":1,"rev":2,"text":"hi","sel":[2,2],"pause":1}"#
        );
        // Files from before the pause count still parse.
        let old: EditBuffer =
            serde_json::from_str(r#"{"session":1,"rev":2,"text":"hi","sel":[2,2]}"#).unwrap();
        assert_eq!(old.pause, 0);
    }

    #[test]
    fn a_command_waits_until_the_buffer_holds_its_revision() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("edit-cmd");
        let buf = dir.path().join("edit-buffer.json");
        let mut reader = CmdReader::default();
        write_buffer_to(&buf, &buffer(1, 4, "old")).unwrap();
        append_cmd_to(&cmds, 1, EditCmd::Resume { rev: 5 }).unwrap();
        append_cmd_to(&cmds, 1, EditCmd::ProxyClosed).unwrap();
        // rev 5 is not written yet: nothing, and the later command waits
        // behind it.
        let read = read_buffer_from(&buf);
        assert_eq!(reader.poll(&cmds, 1, read.as_ref()), []);
        write_buffer_to(&buf, &buffer(1, 5, "new")).unwrap();
        let read = read_buffer_from(&buf);
        assert_eq!(
            reader.poll(&cmds, 1, read.as_ref()),
            [
                Accepted::Resume { text: "new".into() },
                Accepted::ProxyClosed
            ]
        );
        // Each command is read once.
        assert_eq!(reader.poll(&cmds, 1, read.as_ref()), []);
    }

    #[test]
    fn other_sessions_and_half_lines_are_ignored() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("edit-cmd");
        let mut reader = CmdReader::default();
        let b = buffer(2, 1, "text");
        append_cmd_to(&cmds, 1, EditCmd::ProxyClosed).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&cmds)
            .unwrap()
            .write_all(br#"{"session":2,"at_ms":1,"cmd":"res"#)
            .unwrap();
        assert_eq!(reader.poll(&cmds, 2, Some(&b)), []);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&cmds)
            .unwrap()
            .write_all(b"ume\",\"rev\":1}\n")
            .unwrap();
        assert_eq!(
            reader.poll(&cmds, 2, Some(&b)),
            [Accepted::Resume {
                text: "text".into()
            }]
        );
        // A buffer of another session never answers a revision.
        assert_eq!(accept(EditCmd::Resume { rev: 1 }, 3, Some(&b)), None);
    }

    #[test]
    fn a_finish_takes_the_text_at_its_revision() {
        let b = buffer(1, 7, "fixed text");
        assert_eq!(
            accept(
                EditCmd::Finish {
                    mode: FinishMode::Paste,
                    rev: 7
                },
                1,
                Some(&b)
            ),
            Some(Accepted::Finish {
                mode: FinishMode::Paste,
                text: "fixed text".into()
            })
        );
        assert_eq!(
            accept(
                EditCmd::Finish {
                    mode: FinishMode::Paste,
                    rev: 6
                },
                1,
                Some(&b)
            ),
            None
        );
        assert_eq!(accept(EditCmd::Resume { rev: 7 }, 1, None), None);
    }

    #[test]
    fn a_restarted_command_file_is_read_from_its_start() {
        let dir = TempDir::new().unwrap();
        let cmds = dir.path().join("edit-cmd");
        let mut reader = CmdReader::default();
        append_cmd_to(&cmds, 1, EditCmd::ProxyClosed).unwrap();
        append_cmd_to(&cmds, 1, EditCmd::ProxyClosed).unwrap();
        assert_eq!(reader.poll(&cmds, 1, None).len(), 2);
        std::fs::remove_file(&cmds).unwrap();
        append_cmd_to(&cmds, 1, EditCmd::ProxyClosed).unwrap();
        assert_eq!(reader.poll(&cmds, 1, None), [Accepted::ProxyClosed]);
    }
}
