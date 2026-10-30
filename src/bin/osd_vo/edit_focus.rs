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

use vo::scene::input::KeyEvent;
use vo_core::edit::PlainLines;
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
        self.keys.reset();
    }

    /// A key while focused. Returns true when the text box used it.
    pub(crate) fn key(&mut self, ev: &KeyEvent, link: &mut EditLink) -> bool {
        if !self.has || !link.editable() {
            return false;
        }
        let Some(buffer) = link.buffer() else {
            return false;
        };
        let selection = !buffer.selection().is_caret();
        match self.keys.map(ev, selection) {
            None => return false,
            Some(EditCmd::Edit(action)) => {
                let lines = PlainLines::new(buffer.text());
                link.apply(action, &lines, ev.t);
            }
            Some(EditCmd::Collapse) => link.collapse(),
            Some(EditCmd::Resume) => link.resume(),
            Some(EditCmd::Finish) => link.finish(FinishMode::PasteEnter),
        }
        true
    }
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
    }

    impl FocusCtl for Ctl {
        fn take(&mut self) {
            self.log.push("take");
        }
        fn release(&mut self) {
            self.log.push("release");
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
            };
            self.link.sync(Some(&block));
            self.focus.follow(&mut self.link, &mut self.ctl, 0.0);
        }

        fn focus(&mut self, gained: bool, requested: bool) {
            self.focus.changed(gained, requested, &mut self.link, 0.0);
            self.focus.follow(&mut self.link, &mut self.ctl, 0.0);
        }

        fn key(&mut self, key: LogicalKey, text: Option<&str>, state: KeyState) -> bool {
            let ev = KeyEvent {
                surface: 0,
                t: 0.0,
                state,
                key,
                code: 0,
                text: text.map(Into::into),
                mods: Mods::default(),
            };
            let used = self.focus.key(&ev, &mut self.link);
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
}
