//! The app that has keyboard focus, which is where dictated text goes.
//!
//! Wayland gives clients no way to ask this, so each compositor is asked in
//! its own way: `hyprctl` on Hyprland, `swaymsg` on Sway, and `kdotool` on
//! KDE Plasma when it is installed. Anything else gives None.

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

    #[test]
    fn a_missing_tool_gives_none() {
        assert_eq!(run("voxtype-no-such-tool", &[]), None);
    }
}
