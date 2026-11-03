//! Keyboard focus for a keyboard edit: the focus proxy's life (plan R3,
//! R5, R6).
//!
//! The visible popup never takes keys. While the text can be edited the
//! popup asks vo for a focus proxy, a separate small Overlay layer surface
//! that KWin activates when it maps. When editing ends the popup closes it
//! and KWin's focus chain gives focus back to the previous window. Focus the
//! popup did not give up (a click elsewhere, Alt+Tab, the lock screen) moves
//! the edit to Detached: text and caret stay, and only a new edit tap takes
//! focus again. The popup never takes it back by itself.
//!
//! Copy, cut and paste (plan F5) use the clipboard through the focus
//! proxy. Only Ctrl+C, Ctrl+X and their Insert and Delete twins set it:
//! selecting text never does, and the primary selection is never set (R18).
//! Pasted text goes in as typed text.

use vo::scene::input::{KeyEvent, PointerPhase};
use vo_core::edit::{
    line_at, word_at, Affinity, Dir, EditAction, EditBuffer, Lines, PlainLines, Unit,
};
use vo_core::edit_keymap::{EditCmd, TextBoxKeys};
use voxtype::osd::edit_ipc::FinishMode;

use super::edit_link::EditLink;

/// Layer namespace of the focus proxy.
pub(crate) const PROXY_NAMESPACE: &str = "voxtype-edit";

/// Asks for keyboard focus and gives it back: the app's `Cx`, or a test's
/// stand-in.
pub(crate) trait FocusCtl {
    fn take(&mut self);
    fn release(&mut self);
    /// Puts `text` on the clipboard.
    fn copy(&mut self, text: String);
    /// Asks for the clipboard; the app's `clipboard` answers.
    fn paste(&mut self);
}

impl<M> FocusCtl for vo::Cx<'_, M> {
    fn take(&mut self) {
        self.take_keyboard_focus(vo::FocusProxySpec {
            app_id: PROXY_NAMESPACE.into(),
            output: None,
        });
    }

    fn release(&mut self) {
        self.release_keyboard_focus();
    }

    fn copy(&mut self, text: String) {
        vo::Cx::copy(self, text);
    }

    fn paste(&mut self) {
        vo::Cx::paste(self);
    }
}

#[derive(Debug, Default)]
pub(crate) struct Focus {
    /// A proxy was asked for and not given back or taken away since.
    asked: bool,
    /// When it was asked for, to log how long focus took (R40).
    asked_at: Option<f64>,
    /// Keys come now.
    has: bool,
    keys: TextBoxKeys,
    /// A paste was asked for and not answered yet. A focus loss drops it.
    pasting: bool,
    /// A single click is held: a drag extends the selection from it.
    dragging: bool,
}

impl Focus {
    /// After the edit state changed: take focus when the text can be
    /// edited, give it back when it cannot. A detached edit keeps none.
    pub(crate) fn follow(&mut self, link: &mut EditLink, ctl: &mut dyn FocusCtl, t: f64) {
        let want = link.editable();
        if want && !self.asked {
            ctl.take();
            self.asked = true;
            self.asked_at = Some(t);
        } else if !want && self.asked {
            ctl.release();
            self.lost();
            link.proxy_closed();
        }
    }

    /// Focus came or went. A loss the popup did not ask for detaches the
    /// edit.
    pub(crate) fn changed(&mut self, gained: bool, requested: bool, link: &mut EditLink, t: f64) {
        if gained {
            self.has = true;
            if let Some(at) = self.asked_at.take() {
                tracing::info!(ms = (t - at) * 1e3, "keyboard focus for the edit");
            }
            return;
        }
        let was_asked = self.asked;
        self.lost();
        if !requested && was_asked {
            tracing::info!("keyboard focus taken away; the edit is detached");
            link.detach();
        }
    }

    /// R6: held keys and the Ctrl state are forgotten. vo stops the
    /// repeat and the host drops a half-typed compose.
    fn lost(&mut self) {
        self.asked = false;
        self.asked_at = None;
        self.has = false;
        self.pasting = false;
        self.keys.reset();
    }

    /// A key while focused. Returns true when the text box used it.
    /// `lines` is the buffer as the look wraps it, for Up, Down, Home and
    /// End; without it each hard line is one line.
    pub(crate) fn key(
        &mut self,
        ev: &KeyEvent,
        link: &mut EditLink,
        lines: Option<&dyn Lines>,
        ctl: &mut dyn FocusCtl,
    ) -> bool {
        if !self.has || !link.editable() {
            return false;
        }
        let Some(buffer) = link.buffer() else {
            return false;
        };
        let selection = !buffer.selection().is_caret();
        let plain = PlainLines::new(buffer.text());
        let selected = selected(buffer);
        match self.keys.map(ev, selection) {
            None => return false,
            Some(EditCmd::Edit(action)) => {
                link.apply(action, lines.unwrap_or(&plain), ev.t);
            }
            Some(EditCmd::Copy) => {
                if let Some(text) = selected {
                    ctl.copy(text);
                }
            }
            Some(EditCmd::Cut) => {
                if let Some(text) = selected {
                    ctl.copy(text);
                    let delete = EditAction::Delete(Dir::Back, Unit::Grapheme);
                    link.apply(delete, lines.unwrap_or(&plain), ev.t);
                }
            }
            Some(EditCmd::Paste) => {
                self.pasting = true;
                ctl.paste();
            }
            Some(EditCmd::Collapse) => link.collapse(),
            Some(EditCmd::Resume) => link.resume(),
            Some(EditCmd::Finish) => link.finish(FinishMode::PasteEnter),
        }
        true
    }

    /// A press, drag or release of the primary button on the edit text, at
    /// buffer position `at` (plan F6). One click puts the caret there, a
    /// drag from it selects, two clicks select the word and three the line
    /// (`vo_core::edit::word_at`, `line_at`). Returns true when the buffer
    /// changed or its selection moved.
    pub(crate) fn pointer(
        &mut self,
        phase: PointerPhase,
        at: (usize, Affinity),
        link: &mut EditLink,
        lines: Option<&dyn Lines>,
        t: f64,
    ) -> bool {
        if !link.editable() {
            self.dragging = false;
            return false;
        }
        let Some(buffer) = link.buffer() else {
            return false;
        };
        let (to, affinity) = at;
        let select = |r: std::ops::Range<usize>| EditAction::Select {
            anchor: r.start,
            focus: r.end,
        };
        let action = match phase {
            PointerPhase::Press { clicks } => {
                self.dragging = clicks == 1;
                match clicks {
                    1 => EditAction::MoveTo {
                        to,
                        affinity,
                        extend: false,
                    },
                    2 => select(word_at(buffer.text(), to)),
                    _ => select(line_at(buffer.text(), to)),
                }
            }
            PointerPhase::Drag if self.dragging => EditAction::MoveTo {
                to,
                affinity,
                extend: true,
            },
            PointerPhase::Drag => return false,
            PointerPhase::Release => {
                self.dragging = false;
                return false;
            }
        };
        let plain = PlainLines::new(buffer.text());
        link.apply(action, lines.unwrap_or(&plain), t);
        true
    }

    /// The clipboard's answer to a paste. The text goes in at the caret,
    /// replacing any selection, as typed text would. Returns true when the
    /// buffer took it.
    pub(crate) fn pasted(
        &mut self,
        text: Option<String>,
        link: &mut EditLink,
        lines: Option<&dyn Lines>,
        t: f64,
    ) -> bool {
        if !std::mem::take(&mut self.pasting) || !self.has || !link.editable() {
            return false;
        }
        let text = paste_text(&text.unwrap_or_default());
        let Some(buffer) = link.buffer().filter(|_| !text.is_empty()) else {
            return false;
        };
        let plain = PlainLines::new(buffer.text());
        link.apply(EditAction::Insert(text), lines.unwrap_or(&plain), t);
        true
    }
}

/// The selected text, or `None` for a caret.
fn selected(buffer: &EditBuffer) -> Option<String> {
    let sel = buffer.selection();
    let range = sel.anchor.min(sel.focus)..sel.anchor.max(sel.focus);
    (!range.is_empty()).then(|| buffer.text()[range].to_string())
}

/// Pasted text as the text box takes typed text: line breaks become `\n`
/// (Shift+Enter's), a tab a space, and other control characters go.
fn paste_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::edit_link::{Paths, Phase};
    use super::*;
    use tempfile::TempDir;
    use vo::scene::input::{KeyState, LogicalKey, Mods, NamedKey};
    use voxtype::osd::edit_ipc::{Accepted, CmdReader, EditBlock};

    #[derive(Default)]
    struct Ctl {
        log: Vec<&'static str>,
        copied: Vec<String>,
    }

    impl FocusCtl for Ctl {
        fn take(&mut self) {
            self.log.push("take");
        }
        fn release(&mut self) {
            self.log.push("release");
        }
        fn copy(&mut self, text: String) {
            self.copied.push(text);
        }
        fn paste(&mut self) {
            self.log.push("paste");
        }
    }

    struct Rig {
        dir: TempDir,
        link: EditLink,
        focus: Focus,
        ctl: Ctl,
        reader: CmdReader,
    }

    impl Rig {
        fn new() -> Rig {
            let dir = TempDir::new().unwrap();
            let link = EditLink::new(Paths {
                buffer: dir.path().join("edit-buffer.json"),
                cmds: dir.path().join("edit-cmd"),
            });
            Rig {
                dir,
                link,
                focus: Focus::default(),
                ctl: Ctl::default(),
                reader: CmdReader::default(),
            }
        }

        fn daemon(&mut self, phase: &str) {
            let block = EditBlock {
                session: 1,
                phase: phase.into(),
                frozen: "helo".into(),
                tail: String::new(),
                pause: 1,
                raw_words: 0,
                insert_at: None,
            };
            self.link.sync(Some(&block));
            self.focus.follow(&mut self.link, &mut self.ctl, 0.0);
        }

        fn focus(&mut self, gained: bool, requested: bool) {
            self.focus.changed(gained, requested, &mut self.link, 0.0);
            self.focus.follow(&mut self.link, &mut self.ctl, 0.0);
        }

        fn key(&mut self, key: LogicalKey, text: Option<&str>, state: KeyState) -> bool {
            self.key_with(key, text, state, Mods::default())
        }

        fn ctrl(&mut self, c: char) -> bool {
            let mods = Mods {
                ctrl: true,
                ..Mods::default()
            };
            self.key_with(LogicalKey::Char(c), None, KeyState::Down, mods)
        }

        fn key_with(
            &mut self,
            key: LogicalKey,
            text: Option<&str>,
            state: KeyState,
            mods: Mods,
        ) -> bool {
            let ev = KeyEvent {
                surface: 0,
                t: 0.0,
                state,
                key,
                code: 0,
                text: text.map(Into::into),
                mods,
            };
            let used = self.focus.key(&ev, &mut self.link, None, &mut self.ctl);
            self.focus.follow(&mut self.link, &mut self.ctl, 0.0);
            used
        }

        fn sent(&mut self) -> Vec<Accepted> {
            let buffer =
                voxtype::osd::edit_ipc::read_buffer_from(&self.dir.path().join("edit-buffer.json"));
            self.reader
                .poll(&self.dir.path().join("edit-cmd"), 1, buffer.as_ref())
        }
    }

    #[test]
    fn focus_is_taken_once_when_editing_and_given_back_after() {
        let mut rig = Rig::new();
        rig.daemon("pending");
        assert!(
            rig.ctl.log.is_empty(),
            "nothing is taken before the tap is confirmed"
        );
        rig.daemon("settling");
        rig.daemon("editing");
        assert_eq!(rig.ctl.log, ["take"]);
        rig.focus(true, false);
        assert!(rig.key(LogicalKey::Char('l'), Some("l"), KeyState::Down));
        assert_eq!(rig.link.buffer().unwrap().text(), "helol");
        rig.daemon("listening");
        assert_eq!(rig.ctl.log, ["take", "release"]);
        assert_eq!(rig.sent(), [Accepted::ProxyClosed]);
    }

    #[test]
    fn focus_taken_away_detaches_and_only_a_tap_comes_back() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        rig.key(LogicalKey::Char('x'), Some("x"), KeyState::Down);
        // A click elsewhere: the compositor takes focus.
        rig.focus(false, false);
        assert_eq!(rig.link.phase(), Some(Phase::Detached));
        assert_eq!(rig.sent(), [Accepted::Detached]);
        // The daemon's block still says editing until it reads the command:
        // the popup does not take focus back by itself.
        rig.daemon("editing");
        rig.daemon("detached");
        assert_eq!(rig.ctl.log, ["take"]);
        // Keys while detached never reach the text.
        assert!(!rig.key(LogicalKey::Char('y'), Some("y"), KeyState::Down));
        // A tap: the daemon says editing again, the same text and caret come back.
        rig.daemon("editing");
        assert_eq!(rig.ctl.log, ["take", "take"]);
        rig.focus(true, false);
        assert_eq!(rig.link.buffer().unwrap().text(), "helox");
        assert_eq!(rig.link.buffer().unwrap().selection().focus, 5);
    }

    #[test]
    fn a_loss_forgets_held_ctrl_keys() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        let ctrl = LogicalKey::Named(NamedKey::ControlLeft);
        rig.key(ctrl, None, KeyState::Down);
        rig.key(
            LogicalKey::Named(NamedKey::ControlRight),
            None,
            KeyState::Down,
        );
        rig.focus(false, false);
        rig.daemon("detached");
        rig.daemon("editing");
        rig.focus(true, false);
        // Up moves again: the look chord's Ctrl state did not survive the loss.
        rig.key(LogicalKey::Named(NamedKey::Home), None, KeyState::Down);
        assert!(rig.key(LogicalKey::Named(NamedKey::Down), None, KeyState::Down));
    }

    #[test]
    fn esc_resumes_and_enter_finishes_on_release() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        rig.key(LogicalKey::Named(NamedKey::Escape), None, KeyState::Down);
        assert_eq!(rig.ctl.log, ["take", "release"], "focus goes back at once");
        assert!(matches!(
            rig.sent().as_slice(),
            [Accepted::Resume { .. }, Accepted::ProxyClosed]
        ));
        // The daemon's old block does not take focus again.
        rig.daemon("editing");
        assert_eq!(rig.ctl.log, ["take", "release"]);
        rig.daemon("listening");
        rig.daemon("settling");
        rig.focus(true, false);
        let enter = LogicalKey::Named(NamedKey::Enter);
        rig.key(enter, None, KeyState::Down);
        assert!(rig.sent().is_empty(), "Enter acts on release");
        rig.key(enter, None, KeyState::Up);
        assert!(matches!(
            rig.sent().as_slice(),
            [
                Accepted::Finish {
                    mode: FinishMode::PasteEnter,
                    ..
                },
                Accepted::ProxyClosed
            ]
        ));
    }

    #[test]
    fn copy_and_cut_send_the_selection_and_paste_goes_in_as_typed() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        // A caret copies nothing.
        assert!(rig.ctrl('c'));
        assert!(rig.ctl.copied.is_empty());
        // Ctrl+A, Ctrl+C: the whole text; selecting alone copied nothing.
        rig.ctrl('a');
        assert!(
            rig.ctl.copied.is_empty(),
            "a selection never sets the clipboard"
        );
        rig.ctrl('c');
        assert_eq!(rig.ctl.copied, ["helo"]);
        // Ctrl+X: copied and gone.
        rig.ctrl('x');
        assert_eq!(rig.ctl.copied, ["helo", "helo"]);
        assert_eq!(rig.link.buffer().unwrap().text(), "");
        // Ctrl+V asks; the answer goes in, line breaks and tabs as typed.
        rig.ctrl('v');
        assert_eq!(rig.ctl.log, ["take", "paste"]);
        let pasted = Some("tea\r\nfor\ttwo\u{7}".to_string());
        assert!(rig.focus.pasted(pasted, &mut rig.link, None, 0.0));
        assert_eq!(rig.link.buffer().unwrap().text(), "tea\nfor two");
        // An answer nobody asked for is dropped.
        assert!(!rig.focus.pasted(Some("x".into()), &mut rig.link, None, 0.0));
    }

    #[test]
    fn a_paste_answer_after_focus_went_is_dropped() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        rig.ctrl('v');
        rig.focus(false, false);
        assert!(!rig
            .focus
            .pasted(Some("late".into()), &mut rig.link, None, 0.0));
        assert_eq!(rig.link.buffer().unwrap().text(), "helo");
    }

    #[test]
    fn a_click_places_the_caret_a_drag_selects_and_double_and_triple_clicks() {
        let mut rig = Rig::new();
        rig.daemon("editing");
        rig.focus(true, false);
        // "helo" plus a typed line.
        rig.key(LogicalKey::Named(NamedKey::End), None, KeyState::Down);
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        rig.key_with(
            LogicalKey::Named(NamedKey::Enter),
            None,
            KeyState::Down,
            shift,
        );
        for c in ["n", "e", "w", " ", "l", "i", "n", "e"] {
            rig.key(
                LogicalKey::Char(c.chars().next().unwrap()),
                Some(c),
                KeyState::Down,
            );
        }
        assert_eq!(rig.link.buffer().unwrap().text(), "helo\nnew line");
        let sel = |rig: &Rig| rig.link.buffer().unwrap().selection();
        let at = |b: usize| (b, Affinity::Downstream);
        let press = |rig: &mut Rig, phase: PointerPhase, b: usize| {
            rig.focus.pointer(phase, at(b), &mut rig.link, None, 0.0)
        };
        assert!(press(&mut rig, PointerPhase::Press { clicks: 1 }, 2));
        assert!(sel(&rig).is_caret() && sel(&rig).focus == 2);
        assert!(press(&mut rig, PointerPhase::Drag, 9));
        assert_eq!((sel(&rig).anchor, sel(&rig).focus), (2, 9));
        press(&mut rig, PointerPhase::Release, 9);
        assert!(
            !press(&mut rig, PointerPhase::Drag, 3),
            "no drag after release"
        );
        // Two clicks on "line": that word; three: the whole second line.
        press(&mut rig, PointerPhase::Press { clicks: 2 }, 11);
        assert_eq!(sel(&rig).range(), 9..13);
        assert!(
            !press(&mut rig, PointerPhase::Drag, 2),
            "a word click does not drag"
        );
        press(&mut rig, PointerPhase::Press { clicks: 3 }, 11);
        assert_eq!(sel(&rig).range(), 5..13);
        // Detached: clicks do nothing.
        rig.focus(false, false);
        assert!(!press(&mut rig, PointerPhase::Press { clicks: 1 }, 0));
    }
}
