//! The popup's side of the keyboard edit link (`voxtype::osd::edit_ipc`).
//!
//! The daemon publishes the edit phase and the frozen text in the `edit`
//! block of `preview.json`. While a pause lasts the popup owns the text: it
//! keeps an [`EditBuffer`] seeded from the frozen text and writes it, with
//! the selection, to `edit-buffer.json` after every change (one small
//! tmpfs write, never `preview.json`). Resume and Finish go to `edit-cmd`
//! with the revision they take. A popup started again in the same pause
//! reloads its text and caret from that file.

use std::path::PathBuf;

use vo_core::edit::{EditAction, EditBuffer, EditEffect, Lines, Selection};
use voxtype::osd::edit_ipc::{self, EditBlock, EditCmd, FinishMode};

/// The edit phase as the daemon publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Listening,
    Pending,
    Settling,
    Editing,
    Detached,
    Releasing,
}

impl Phase {
    fn parse(s: &str) -> Option<Phase> {
        Some(match s {
            "listening" => Phase::Listening,
            "pending" => Phase::Pending,
            "settling" => Phase::Settling,
            "editing" => Phase::Editing,
            "detached" => Phase::Detached,
            "releasing" => Phase::Releasing,
            _ => return None,
        })
    }

    /// The popup owns the text: it may be edited, or is kept for a re-entry.
    pub(crate) fn owns_text(self) -> bool {
        matches!(self, Phase::Settling | Phase::Editing | Phase::Detached)
    }
}

/// Where the link reads and writes: the runtime folder, or a test's own.
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    pub buffer: PathBuf,
    pub cmds: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Paths {
            buffer: edit_ipc::buffer_path(),
            cmds: edit_ipc::cmd_path(),
        }
    }
}

/// What a new edit block changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Nothing,
    /// The phase changed and the text did not.
    Phase,
    /// A pause began: the buffer is new (seeded, or reloaded after a
    /// restart).
    Entered,
    /// The settling tail landed at the end of the buffer.
    Tail,
    /// No edit any more: the buffer is gone.
    Left,
}

/// The popup's edit state for one dictation.
#[derive(Debug, Default)]
pub(crate) struct EditLink {
    /// None for a link that reads and writes no files (`EditLink::offline`).
    paths: Option<Paths>,
    session: u64,
    pause: u32,
    phase: Option<Phase>,
    buffer: Option<EditBuffer>,
    /// The frozen text the buffer was seeded from, so a settled tail can be
    /// added to it.
    seeded: String,
    /// Words still settling, drawn after the buffer and not editable.
    tail: String,
    /// The revision in the buffer file. Goes up with each write and never
    /// down within a session, also across a restart.
    rev: u64,
    /// The buffer and selection last written.
    written: Option<(u64, Selection)>,
    /// The daemon's phase when the popup moved on by itself (resume,
    /// finish, focus lost). The popup's phase stands until the daemon
    /// publishes another.
    ahead_of: Option<Phase>,
}

impl EditLink {
    pub(crate) fn new(paths: Paths) -> EditLink {
        EditLink {
            paths: Some(paths),
            ..EditLink::default()
        }
    }

    /// A link that reads and writes no files, for scripted moments
    /// (`--moment edit`): the daemon's side comes from the script.
    pub(crate) fn offline() -> EditLink {
        EditLink::default()
    }

    pub(crate) fn phase(&self) -> Option<Phase> {
        self.phase
    }

    pub(crate) fn buffer(&self) -> Option<&EditBuffer> {
        self.buffer.as_ref()
    }

    pub(crate) fn tail(&self) -> &str {
        &self.tail
    }

    /// The text can take keys: paused, settled or not, and focused.
    pub(crate) fn editable(&self) -> bool {
        self.buffer.is_some() && matches!(self.phase, Some(Phase::Settling | Phase::Editing))
    }

    /// `preview.json` changed; `block` is its edit block.
    pub(crate) fn sync(&mut self, block: Option<&EditBlock>) -> Change {
        let Some(block) = block else {
            return self.leave();
        };
        if block.session != self.session {
            self.leave();
            self.session = block.session;
            self.rev = 0;
        }
        let phase = Phase::parse(&block.phase);
        if phase.is_some() && phase == self.ahead_of {
            return Change::Nothing;
        }
        self.ahead_of = None;
        let was = std::mem::replace(&mut self.phase, phase);
        let Some(phase) = phase.filter(|p| p.owns_text()) else {
            self.tail.clear();
            return if was == phase {
                Change::Nothing
            } else {
                Change::Phase
            };
        };
        self.tail = if phase == Phase::Settling {
            block.tail.clone()
        } else {
            String::new()
        };
        if self.buffer.is_none() || block.pause != self.pause {
            self.enter(block);
            return Change::Entered;
        }
        if block.frozen != self.seeded && self.add_tail(&block.frozen) {
            return Change::Tail;
        }
        if was == Some(phase) {
            Change::Nothing
        } else {
            Change::Phase
        }
    }

    fn leave(&mut self) -> Change {
        self.phase = None;
        self.ahead_of = None;
        self.tail.clear();
        self.written = None;
        if self.buffer.take().is_some() {
            Change::Left
        } else {
            Change::Nothing
        }
    }

    /// A pause began, or the popup started during one: reload the snapshot
    /// of this pause when there is one, else seed from the frozen text.
    fn enter(&mut self, block: &EditBlock) {
        self.pause = block.pause;
        self.seeded = block.frozen.clone();
        if let Some(buffer) = self.buffer.as_mut() {
            // A new pause of a running popup: the frozen text holds the
            // last edit and the speech since; an undo floor starts here.
            buffer.seed(block.frozen.clone());
        } else if let Some(snap) = self
            .paths
            .as_ref()
            .and_then(|p| edit_ipc::read_buffer_from(&p.buffer))
            .filter(|b| b.session == block.session && b.pause == block.pause)
        {
            let mut buffer = EditBuffer::new(snap.text);
            buffer.set_selection(Selection {
                anchor: snap.sel.0,
                focus: snap.sel.1,
                ..Selection::default()
            });
            self.rev = self.rev.max(snap.rev);
            self.written = Some((buffer.rev(), buffer.selection()));
            self.buffer = Some(buffer);
        } else {
            self.buffer = Some(EditBuffer::new(block.frozen.clone()));
        }
        self.write();
    }

    /// The tail settled: the frozen text grew at its end. The buffer gets
    /// the new words at its end. A caret at the end moves to the new end; a
    /// caret moved elsewhere stays. The added words cannot be undone.
    fn add_tail(&mut self, frozen: &str) -> bool {
        let Some(added) = frozen.strip_prefix(self.seeded.as_str()) else {
            return false;
        };
        let added = added.to_string();
        self.seeded = frozen.to_string();
        let Some(buffer) = self.buffer.as_mut() else {
            return false;
        };
        let sel = buffer.selection();
        let at_end = sel.is_caret() && sel.focus == buffer.text().len();
        buffer.set_selection(Selection::caret(buffer.text().len()));
        buffer.apply_plain(EditAction::Insert(added), 0.0);
        buffer.set_floor();
        if !at_end {
            buffer.set_selection(sel);
        }
        self.write();
        true
    }

    /// Do `action` at `t` while the text can take keys. Writes the buffer
    /// file when the text or the selection changed.
    pub(crate) fn apply(
        &mut self,
        action: EditAction,
        lines: &dyn Lines,
        t: f64,
    ) -> Option<EditEffect> {
        if !self.editable() {
            return None;
        }
        let effect = self.buffer.as_mut()?.apply(action, lines, t);
        self.write();
        effect
    }

    /// Write the buffer file if the text or selection changed since the
    /// last write.
    fn write(&mut self) {
        let Some(buffer) = &self.buffer else {
            return;
        };
        let now = (buffer.rev(), buffer.selection());
        if self.written == Some(now) {
            return;
        }
        self.rev += 1;
        let snap = edit_ipc::EditBuffer {
            session: self.session,
            rev: self.rev,
            text: buffer.text().to_string(),
            sel: (now.1.anchor, now.1.focus),
            pause: self.pause,
        };
        let Some(paths) = &self.paths else {
            self.written = Some(now);
            return;
        };
        match edit_ipc::write_buffer_to(&paths.buffer, &snap) {
            Ok(()) => self.written = Some(now),
            Err(e) => tracing::warn!("could not write the edit buffer: {e}"),
        }
    }

    fn send(&mut self, cmd: EditCmd) {
        let Some(paths) = &self.paths else {
            return;
        };
        if let Err(e) = edit_ipc::append_cmd_to(&paths.cmds, self.session, cmd) {
            tracing::warn!("could not send {cmd:?} to the daemon: {e}");
        }
    }

    /// Collapse the selection to its focus (Esc with a selection).
    pub(crate) fn collapse(&mut self) {
        if let Some(buffer) = self.buffer.as_mut() {
            let focus = buffer.selection().focus;
            buffer.set_selection(Selection::caret(focus));
            self.write();
        }
    }

    /// Hand the text back and listen again. The text takes no more keys
    /// until the daemon says how it goes on.
    pub(crate) fn resume(&mut self) {
        if self.buffer.is_some() {
            self.write();
            self.send(EditCmd::Resume { rev: self.rev });
            self.move_on(Phase::Releasing);
        }
    }

    /// Hand the text back and deliver it.
    pub(crate) fn finish(&mut self, mode: FinishMode) {
        if self.buffer.is_some() {
            self.write();
            self.send(EditCmd::Finish {
                mode,
                rev: self.rev,
            });
            self.move_on(Phase::Releasing);
        }
    }

    /// The compositor took focus away while editing: keep the text and
    /// caret, stay paused, and tell the daemon.
    pub(crate) fn detach(&mut self) {
        if self.editable() {
            self.move_on(Phase::Detached);
            self.send(EditCmd::Detached);
        }
    }

    fn move_on(&mut self, phase: Phase) {
        self.ahead_of = self.ahead_of.or(self.phase);
        self.phase = Some(phase);
    }

    /// The popup destroyed its focus proxy.
    pub(crate) fn proxy_closed(&mut self) {
        self.send(EditCmd::ProxyClosed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vo_core::edit::{Dir, PlainLines, Unit};
    use voxtype::osd::edit_ipc::{read_buffer_from, Accepted, CmdReader};

    fn paths(dir: &TempDir) -> Paths {
        Paths {
            buffer: dir.path().join("edit-buffer.json"),
            cmds: dir.path().join("edit-cmd"),
        }
    }

    fn block(phase: &str, frozen: &str, pause: u32) -> EditBlock {
        EditBlock {
            session: 7,
            phase: phase.into(),
            frozen: frozen.into(),
            tail: String::new(),
            pause,
            raw_words: 0,
        }
    }

    fn act(link: &mut EditLink, action: EditAction) {
        let lines = PlainLines::new(link.buffer().unwrap().text());
        link.apply(action, &lines, 0.0);
    }

    fn back(link: &mut EditLink, n: usize) {
        for _ in 0..n {
            act(
                link,
                EditAction::MoveBy {
                    dir: Dir::Back,
                    unit: Unit::Grapheme,
                    extend: false,
                },
            );
        }
    }

    #[test]
    fn a_restarted_popup_reloads_the_buffer_and_caret() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        assert_eq!(
            link.sync(Some(&block("editing", "helo world", 1))),
            Change::Entered
        );
        back(&mut link, 7);
        act(&mut link, EditAction::Insert("l".into()));
        assert_eq!(link.buffer().unwrap().text(), "hello world");
        let caret = link.buffer().unwrap().selection();
        assert_eq!(caret, Selection::caret(4));
        drop(link);

        // The popup is started again during the same pause.
        let mut again = EditLink::new(paths(&dir));
        assert_eq!(
            again.sync(Some(&block("editing", "helo world", 1))),
            Change::Entered
        );
        assert_eq!(again.buffer().unwrap().text(), "hello world");
        assert_eq!(again.buffer().unwrap().selection(), caret);
        // Its next write carries on the revision.
        let before = read_buffer_from(&dir.path().join("edit-buffer.json"))
            .unwrap()
            .rev;
        act(&mut again, EditAction::Insert("!".into()));
        let after = read_buffer_from(&dir.path().join("edit-buffer.json")).unwrap();
        assert_eq!(after.rev, before + 1);
        assert_eq!(after.text, "hell!o world");
    }

    #[test]
    fn a_selection_survives_a_restart() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("editing", "one two", 1)));
        act(&mut link, EditAction::SelectAll);
        let mut again = EditLink::new(paths(&dir));
        again.sync(Some(&block("detached", "one two", 1)));
        assert_eq!(again.buffer().unwrap().selection().range(), 0..7);
    }

    #[test]
    fn a_snapshot_of_another_pause_or_dictation_is_not_reloaded() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("editing", "old", 1)));
        act(&mut link, EditAction::Insert(" typed".into()));
        // A later pause: the daemon's frozen text has the edit and new
        // speech, and is newer than the file.
        let mut again = EditLink::new(paths(&dir));
        again.sync(Some(&block("settling", "old typed and more", 2)));
        assert_eq!(again.buffer().unwrap().text(), "old typed and more");
        // Another dictation.
        let mut other = EditLink::new(paths(&dir));
        other.sync(Some(&EditBlock {
            session: 8,
            ..block("editing", "fresh", 2)
        }));
        assert_eq!(other.buffer().unwrap().text(), "fresh");
    }

    #[test]
    fn a_new_pause_reseeds_with_an_undo_floor() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("editing", "one", 1)));
        act(&mut link, EditAction::Insert(" two".into()));
        assert_eq!(
            link.sync(Some(&block("listening", "one two", 1))),
            Change::Phase
        );
        assert!(!link.editable());
        assert_eq!(
            link.sync(Some(&block("settling", "one two three", 2))),
            Change::Entered
        );
        assert_eq!(link.buffer().unwrap().text(), "one two three");
        assert!(!link.buffer().unwrap().can_undo());
    }

    #[test]
    fn the_settled_tail_is_added_at_the_end() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        let mut settling = block("settling", "the quick", 1);
        settling.tail = "brown fox".into();
        link.sync(Some(&settling));
        assert_eq!(link.tail(), "brown fox");
        assert_eq!(
            link.sync(Some(&block("editing", "the quick brown fox", 1))),
            Change::Tail
        );
        assert_eq!(link.tail(), "");
        assert_eq!(link.buffer().unwrap().text(), "the quick brown fox");
        // The caret was at the end, so it moves to the new end, and the
        // added words cannot be undone.
        assert_eq!(link.buffer().unwrap().selection(), Selection::caret(19));
        assert!(!link.buffer().unwrap().can_undo());

        // A caret moved while settling stays put.
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("settling", "the quick", 1)));
        back(&mut link, 6);
        link.sync(Some(&block("editing", "the quick brown fox", 1)));
        assert_eq!(link.buffer().unwrap().selection(), Selection::caret(3));
        let file = read_buffer_from(&dir.path().join("edit-buffer.json")).unwrap();
        assert_eq!(file.text, "the quick brown fox");
    }

    #[test]
    fn one_write_per_change_and_none_without_one() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("edit-buffer.json");
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("editing", "ab", 1)));
        let rev = read_buffer_from(&file).unwrap().rev;
        // A move right at the end changes nothing: no write.
        act(
            &mut link,
            EditAction::MoveBy {
                dir: Dir::Forward,
                unit: Unit::Grapheme,
                extend: false,
            },
        );
        assert_eq!(read_buffer_from(&file).unwrap().rev, rev);
        act(&mut link, EditAction::Insert("c".into()));
        assert_eq!(read_buffer_from(&file).unwrap().rev, rev + 1);
        back(&mut link, 1);
        let snap = read_buffer_from(&file).unwrap();
        assert_eq!((snap.rev, snap.sel), (rev + 2, (2, 2)));
    }

    #[test]
    fn keys_do_nothing_when_detached_or_listening() {
        let dir = TempDir::new().unwrap();
        let mut link = EditLink::new(paths(&dir));
        link.sync(Some(&block("detached", "kept", 1)));
        act(&mut link, EditAction::Insert("x".into()));
        assert_eq!(link.buffer().unwrap().text(), "kept");
        assert_eq!(link.sync(None), Change::Left);
        assert!(link.buffer().is_none());
    }

    #[test]
    fn commands_carry_the_revision_the_daemon_takes() {
        let dir = TempDir::new().unwrap();
        let p = paths(&dir);
        let mut link = EditLink::new(p.clone());
        link.sync(Some(&block("editing", "fix", 1)));
        act(&mut link, EditAction::Insert("ed".into()));
        link.resume();
        link.finish(FinishMode::PasteEnter);
        link.proxy_closed();
        let mut reader = CmdReader::default();
        let buffer = read_buffer_from(&p.buffer);
        assert_eq!(
            reader.poll(&p.cmds, 7, buffer.as_ref()),
            vec![
                Accepted::Resume {
                    text: "fixed".into()
                },
                Accepted::Finish {
                    mode: FinishMode::PasteEnter,
                    text: "fixed".into()
                },
                Accepted::ProxyClosed,
            ]
        );
    }
}
