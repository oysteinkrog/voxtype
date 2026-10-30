//! The app that has keyboard focus, which is where dictated text goes.
//!
//! Wayland gives clients no way to ask this, so each compositor is asked in
//! its own way: `hyprctl` on Hyprland, `swaymsg` on Sway, and `kdotool` on
//! KDE Plasma when it is installed. Anything else gives None.
//!
//! A keyboard edit also needs the window itself, not just its app: the
//! daemon saves the active window's id when the recording starts and pastes
//! only when that same window is active again (plan R26 to R29). Two windows
//! of one app are different targets. This uses `kdotool`, so outside KDE
//! Plasma an edited dictation always goes to the clipboard.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a compositor tool may take before the answer is dropped.
const TIMEOUT: Duration = Duration::from_millis(800);

/// The focused window's app id or window class, or None when it cannot be
/// found out here.
pub fn focused_app() -> Option<String> {
    let app = if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        run("hyprctl", &["activewindow", "-j"]).and_then(|out| hyprland_class(&out))
    } else if std::env::var_os("SWAYSOCK").is_some() {
        run("swaymsg", &["-t", "get_tree", "-r"]).and_then(|out| sway_focused(&out))
    } else {
        run("kdotool", &["getactivewindow", "getwindowclassname"])
    };
    app.map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
}

/// A window that dictated text goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// KWin's id for the window, such as `{18f730db-...}`.
    pub id: String,
    pub class: String,
}

/// The window tool failed or timed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolFailed;

/// Window queries for the paste gate, so it can be tested without a
/// compositor.
pub trait Windows: Send {
    /// The active window's id. None when no window is active, or when the
    /// tool failed or timed out.
    fn active(&mut self) -> Option<String>;
    /// The class of window `id`, `Ok(None)` when the window is closed.
    fn class_of(&mut self, id: &str) -> Result<Option<String>, ToolFailed>;
    /// Ask the compositor to activate window `id`.
    fn activate(&mut self, id: &str);
}

/// [`Windows`] through `kdotool`.
pub struct Kdotool;

impl Windows for Kdotool {
    fn active(&mut self) -> Option<String> {
        run("kdotool", &["getactivewindow"])
            .map(|out| out.trim().to_string())
            .filter(|id| !id.is_empty())
    }

    fn class_of(&mut self, id: &str) -> Result<Option<String>, ToolFailed> {
        // A closed window's class comes back empty, with success.
        let out = run("kdotool", &["getwindowclassname", id]).ok_or(ToolFailed)?;
        let class = out.trim();
        Ok((!class.is_empty()).then(|| class.to_string()))
    }

    fn activate(&mut self, id: &str) {
        let _ = run("kdotool", &["windowactivate", id]);
    }
}

/// The active window, or None when it cannot be found out.
pub fn active_window(w: &mut dyn Windows) -> Option<Window> {
    let id = w.active()?;
    let class = w.class_of(&id).ok()??;
    Some(Window { id, class })
}

/// What the paste gate found out about the saved window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The saved window is the active one.
    Confirmed,
    /// Another window is active, or none is. A window of the same class is
    /// another window.
    Mismatch,
    /// The saved window was closed.
    Gone,
    /// The saved window did not become active in time.
    Timeout,
    /// No window was saved, or the tool failed.
    Failed,
}

impl Verdict {
    pub fn confirmed(self) -> bool {
        self == Verdict::Confirmed
    }

    /// Why the text was not pasted, for the log and the popup.
    pub fn reason(self) -> &'static str {
        match self {
            Verdict::Confirmed => "",
            Verdict::Mismatch => "another window is active",
            Verdict::Gone => "the window was closed",
            Verdict::Timeout => "the window did not get focus back",
            Verdict::Failed => "the window could not be checked",
        }
    }
}

/// How long the window may take to get focus back after an edit.
pub const HAND_BACK: Duration = Duration::from_millis(500);
/// With no window active this long after the hand-back starts, the focus
/// chain gave focus to nothing (the proxy was on another output).
const ACTIVATE_AFTER: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(10);

/// Wait until the saved window is active again, for at most `limit` (R27).
/// When another window got focus, or none did by `ACTIVATE_AFTER`, the
/// saved window is activated, once.
pub fn hand_back(w: &mut dyn Windows, saved: &str, limit: Duration) -> Verdict {
    let start = Instant::now();
    hand_back_with(w, saved, limit, &mut || start.elapsed(), &mut |d| {
        std::thread::sleep(d)
    })
}

fn hand_back_with(
    w: &mut dyn Windows,
    saved: &str,
    limit: Duration,
    elapsed: &mut dyn FnMut() -> Duration,
    sleep: &mut dyn FnMut(Duration),
) -> Verdict {
    let mut activated = false;
    loop {
        let active = w.active();
        if active.as_deref() == Some(saved) {
            return Verdict::Confirmed;
        }
        if elapsed() >= limit {
            return match w.class_of(saved) {
                Ok(None) => Verdict::Gone,
                Ok(Some(_)) => Verdict::Timeout,
                Err(ToolFailed) => Verdict::Failed,
            };
        }
        if !activated && (active.is_some() || elapsed() >= ACTIVATE_AFTER) {
            match w.class_of(saved) {
                Ok(None) => return Verdict::Gone,
                Err(ToolFailed) => return Verdict::Failed,
                Ok(Some(_)) => {}
            }
            activated = true;
            w.activate(saved);
        }
        sleep(POLL);
    }
}

/// Is the saved window still the active one (R28)?
pub fn check(w: &mut dyn Windows, saved: &str) -> Verdict {
    if w.active().as_deref() == Some(saved) {
        return Verdict::Confirmed;
    }
    match w.class_of(saved) {
        Ok(None) => Verdict::Gone,
        Ok(Some(_)) => Verdict::Mismatch,
        Err(ToolFailed) => Verdict::Failed,
    }
}

/// Runs `cmd`, and returns its output when it succeeds within `TIMEOUT`.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut()?, &mut out).ok()?;
    Some(out)
}

fn hyprland_class(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(v["class"].as_str()?.to_string())
}

/// The focused node of a `swaymsg -t get_tree` tree: its Wayland app id, or
/// its X11 class under Xwayland.
fn sway_focused(json: &str) -> Option<String> {
    fn find(node: &serde_json::Value) -> Option<String> {
        if node["focused"].as_bool() == Some(true) {
            let app = node["app_id"]
                .as_str()
                .or_else(|| node["window_properties"]["class"].as_str())?;
            return Some(app.to_string());
        }
        ["nodes", "floating_nodes"]
            .iter()
            .filter_map(|k| node[*k].as_array())
            .flatten()
            .find_map(find)
    }
    find(&serde_json::from_str(json).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_gives_the_window_class() {
        let json = r#"{"address": "0x1", "class": "firefox", "title": "Some page"}"#;
        assert_eq!(hyprland_class(json).as_deref(), Some("firefox"));
        assert_eq!(hyprland_class("{}"), None);
    }

    #[test]
    fn sway_finds_the_focused_node_at_any_depth() {
        let json = r#"{"focused": false, "nodes": [
            {"focused": false, "nodes": [{"focused": false, "app_id": "foot"}]},
            {"focused": false, "floating_nodes": [
                {"focused": true, "app_id": null, "window_properties": {"class": "Gimp"}}]}]}"#;
        assert_eq!(sway_focused(json).as_deref(), Some("Gimp"));
    }

    /// Windows by id and class; `focus_on_activate` is what the
    /// compositor does when asked to activate a window.
    struct Fake {
        windows: Vec<(&'static str, &'static str)>,
        active: Option<&'static str>,
        broken: bool,
        focus_on_activate: bool,
        activated: Vec<String>,
    }

    impl Fake {
        fn new(windows: &[(&'static str, &'static str)], active: Option<&'static str>) -> Self {
            Self {
                windows: windows.to_vec(),
                active,
                broken: false,
                focus_on_activate: true,
                activated: Vec::new(),
            }
        }
    }

    impl Windows for Fake {
        fn active(&mut self) -> Option<String> {
            (!self.broken).then(|| self.active.map(str::to_string))?
        }
        fn class_of(&mut self, id: &str) -> Result<Option<String>, ToolFailed> {
            if self.broken {
                return Err(ToolFailed);
            }
            Ok(self
                .windows
                .iter()
                .find(|(w, _)| *w == id)
                .map(|(_, c)| c.to_string()))
        }
        fn activate(&mut self, id: &str) {
            self.activated.push(id.to_string());
            if self.focus_on_activate {
                self.active = self.windows.iter().find(|(w, _)| *w == id).map(|(w, _)| *w);
            }
        }
    }

    /// Runs the hand-back on a clock that only moves when it sleeps.
    fn hand_back_fake(w: &mut Fake, saved: &str) -> Verdict {
        let now = std::cell::Cell::new(Duration::ZERO);
        hand_back_with(w, saved, HAND_BACK, &mut || now.get(), &mut |d| {
            now.set(now.get() + d)
        })
    }

    #[test]
    fn the_saved_window_already_active_is_confirmed_at_once() {
        let mut w = Fake::new(&[("a", "term")], Some("a"));
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Confirmed);
        assert!(w.activated.is_empty());
    }

    #[test]
    fn another_window_with_focus_is_activated_away_once() {
        let mut w = Fake::new(&[("a", "term"), ("b", "term")], Some("b"));
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Confirmed);
        assert_eq!(w.activated, ["a"]);
    }

    #[test]
    fn a_window_that_never_gets_focus_times_out_after_one_activation() {
        let mut w = Fake::new(&[("a", "term"), ("b", "term")], Some("b"));
        w.focus_on_activate = false;
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Timeout);
        assert_eq!(w.activated, ["a"], "activated once, never again");
    }

    #[test]
    fn no_window_active_activates_after_a_while() {
        let mut w = Fake::new(&[("a", "term")], None);
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Confirmed);
        assert_eq!(w.activated, ["a"]);
    }

    #[test]
    fn a_closed_window_is_gone_and_never_activated() {
        let mut w = Fake::new(&[("b", "term")], Some("b"));
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Gone);
        assert!(w.activated.is_empty());
        assert_eq!(check(&mut w, "a"), Verdict::Gone);
    }

    #[test]
    fn a_window_of_the_same_class_is_not_a_match() {
        let mut w = Fake::new(&[("a", "term"), ("b", "term")], Some("b"));
        assert_eq!(check(&mut w, "a"), Verdict::Mismatch);
        assert!(w.activated.is_empty(), "the check never activates");
        w.active = Some("a");
        assert_eq!(check(&mut w, "a"), Verdict::Confirmed);
    }

    #[test]
    fn a_failing_tool_never_confirms() {
        let mut w = Fake::new(&[("a", "term")], Some("a"));
        w.broken = true;
        assert_eq!(check(&mut w, "a"), Verdict::Failed);
        assert_eq!(hand_back_fake(&mut w, "a"), Verdict::Failed);
        assert_eq!(active_window(&mut w), None);
    }

    #[test]
    fn the_active_window_has_its_id_and_class() {
        let mut w = Fake::new(&[("a", "term")], Some("a"));
        assert_eq!(
            active_window(&mut w),
            Some(Window {
                id: "a".into(),
                class: "term".into()
            })
        );
    }

    #[test]
    fn a_missing_tool_gives_none() {
        assert_eq!(run("voxtype-no-such-tool", &[]), None);
    }
}
