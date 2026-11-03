//! Detects whether any modifier key is currently held, to avoid wtype/dotool/
//! ydotool/eitype synthesizing keystrokes that the compositor interprets as
//! keybindings (e.g. user is still pressing Super+Ctrl when transcription
//! completes).
//!
//! Uses `EVIOCGKEY` (via `evdev::Device::get_key_state`) to take a passive
//! snapshot of currently-pressed keys without consuming events. Works on any
//! Wayland compositor or X11 because it bypasses the display server.
//!
//! Degrades gracefully when `/dev/input` is not readable (user not in the
//! `input` group, no keyboard devices present, or all devices return permission
//! errors): the guard becomes `Disabled` and `wait_for_release` returns
//! immediately so output proceeds as before.
//!
//! [`wait_until_released`] keeps the opened devices for up to 30 s, and
//! [`prewarm`] opens them again ahead of a paste, so a keyboard plugged in
//! mid-session is picked up without inotify watchers in the output path.
//! Opening them takes 50 ms or more (17 devices, some slow to open), which
//! used to come before every paste.

use evdev::{AttributeSet, Device, KeyCode as Key};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Modifier keys whose presence we treat as "do not type yet". All of these
/// are routinely combined with letter keys to form compositor or application
/// keybindings.
const MODIFIER_KEYS: &[Key] = &[
    Key::KEY_LEFTCTRL,
    Key::KEY_RIGHTCTRL,
    Key::KEY_LEFTALT,
    Key::KEY_RIGHTALT,
    Key::KEY_LEFTSHIFT,
    Key::KEY_RIGHTSHIFT,
    Key::KEY_LEFTMETA,
    Key::KEY_RIGHTMETA,
];

/// Enter keys. A held Enter that finished a keyboard edit would repeat
/// into the target window, or turn the paste keystroke into another
/// shortcut, so the paste waits for it too (plan R30).
const ENTER_KEYS: &[Key] = &[Key::KEY_ENTER, Key::KEY_KPENTER];

/// Keys that must be up before keystrokes are synthesized.
fn blocks_output(state: &AttributeSet<Key>) -> bool {
    MODIFIER_KEYS
        .iter()
        .chain(ENTER_KEYS)
        .any(|k| state.contains(*k))
}

/// Snapshot-based modifier-state checker.
pub enum ModifierGuard {
    /// At least one keyboard event device is readable; modifier checks are live.
    Active { devices: Vec<Device> },
    /// No readable keyboard devices (typically: user not in the `input` group).
    /// All checks short-circuit to "no modifier held" so output proceeds.
    Disabled,
}

impl Default for ModifierGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ModifierGuard {
    /// Probe `/dev/input/event*` and open every device that looks like a
    /// keyboard. Permission errors are silently downgraded to `Disabled` so
    /// that users without `input` group membership get the same behavior they
    /// had before this feature existed.
    pub fn new() -> Self {
        let entries = match std::fs::read_dir("/dev/input") {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "modifier_guard: /dev/input not readable, disabling \
                     (likely missing 'input' group membership)"
                );
                return Self::Disabled;
            }
        };

        let mut devices = Vec::new();
        for entry in entries.flatten() {
            let path: PathBuf = entry.path();
            let is_event_device = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("event"))
                .unwrap_or(false);
            if !is_event_device {
                continue;
            }

            // Per-device permission errors are common (e.g. some event nodes
            // are owned by other groups); skip silently.
            let Ok(device) = Device::open(&path) else {
                continue;
            };

            let is_keyboard = device
                .supported_keys()
                .map(|keys| {
                    keys.contains(Key::KEY_A)
                        && keys.contains(Key::KEY_Z)
                        && keys.contains(Key::KEY_ENTER)
                })
                .unwrap_or(false);
            if is_keyboard {
                devices.push(device);
            }
        }

        if devices.is_empty() {
            tracing::debug!("modifier_guard: no readable keyboard devices, disabling");
            Self::Disabled
        } else {
            tracing::debug!(
                count = devices.len(),
                "modifier_guard: tracking keyboard device(s)"
            );
            Self::Active { devices }
        }
    }

    /// Returns `true` if any modifier key, or Enter, is currently held on
    /// any tracked keyboard. Always returns `false` when `Disabled`.
    pub fn any_modifier_held(&mut self) -> bool {
        let Self::Active { devices } = self else {
            return false;
        };
        for device in devices.iter_mut() {
            // EVIOCGKEY returns the current pressed-key bitmap; cheap and does
            // not consume queued events.
            let state: AttributeSet<Key> = match device.get_key_state() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if blocks_output(&state) {
                return true;
            }
        }
        false
    }

    /// Block until no modifier is held, or until `timeout` elapses. Returns
    /// `Ok(())` on clear release (or immediately when `Disabled`), and
    /// `Err(Timeout)` if modifiers were still held when the timeout expired.
    pub async fn wait_for_release(&mut self, timeout: Duration) -> Result<(), Timeout> {
        if matches!(self, Self::Disabled) {
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        let poll = Duration::from_millis(15);
        while self.any_modifier_held() {
            if Instant::now() >= deadline {
                return Err(Timeout);
            }
            tokio::time::sleep(poll).await;
        }
        Ok(())
    }
}

/// Devices opened by [`prewarm`] or a [`wait_until_released`], with when.
static OPENED: std::sync::Mutex<Option<(Instant, ModifierGuard)>> = std::sync::Mutex::new(None);

/// How long opened devices are reused before they are opened again.
const REUSE_FOR: Duration = Duration::from_secs(30);

/// Open the keyboard devices now, on a thread of their own, for the next
/// [`wait_until_released`]. For the moment a recording stops: the text is
/// at least a transcription away.
pub fn prewarm() {
    std::thread::spawn(|| {
        let guard = ModifierGuard::new();
        *OPENED.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), guard));
    });
}

/// [`ModifierGuard::wait_for_release`] on the devices [`prewarm`] or an
/// earlier call opened, when that was less than 30 s ago; otherwise they
/// are opened now, off the async threads.
pub async fn wait_until_released(timeout: Duration) -> Result<(), Timeout> {
    let kept = OPENED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .filter(|(at, _)| at.elapsed() < REUSE_FOR);
    let (at, mut guard) = match kept {
        Some(kept) => kept,
        None => {
            let guard = tokio::task::spawn_blocking(ModifierGuard::new)
                .await
                .unwrap_or(ModifierGuard::Disabled);
            (Instant::now(), guard)
        }
    };
    let result = guard.wait_for_release(timeout).await;
    let mut slot = OPENED.lock().unwrap_or_else(|e| e.into_inner());
    // A prewarm meanwhile is newer.
    if slot.is_none() {
        *slot = Some((at, guard));
    }
    result
}

/// Returned when `wait_for_release` gives up because modifiers were still held
/// past the configured timeout.
#[derive(Debug, Clone, Copy)]
pub struct Timeout;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disabled_guard_returns_immediately() {
        let mut guard = ModifierGuard::Disabled;
        let start = Instant::now();
        let res = guard.wait_for_release(Duration::from_secs(60)).await;
        assert!(res.is_ok());
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "Disabled guard should not actually wait"
        );
    }

    #[tokio::test]
    async fn disabled_guard_reports_no_modifier_held() {
        let mut guard = ModifierGuard::Disabled;
        assert!(!guard.any_modifier_held());
    }

    fn held(keys: &[Key]) -> bool {
        let mut state = AttributeSet::<Key>::new();
        for k in keys {
            state.insert(*k);
        }
        blocks_output(&state)
    }

    #[test]
    fn enter_and_the_finishing_keys_hold_the_output() {
        // The keys that finish a keyboard edit: Enter, and the Right Ctrl
        // and Right Shift double taps.
        for k in [
            Key::KEY_ENTER,
            Key::KEY_KPENTER,
            Key::KEY_RIGHTCTRL,
            Key::KEY_RIGHTSHIFT,
        ] {
            assert!(held(&[k]), "{k:?} held must wait");
        }
        assert!(held(&[Key::KEY_A, Key::KEY_LEFTMETA]));
    }

    #[test]
    fn other_keys_do_not_hold_the_output() {
        assert!(!held(&[]));
        assert!(!held(&[Key::KEY_A, Key::KEY_SPACE, Key::KEY_ESC]));
    }

    #[test]
    fn new_does_not_panic_without_permission() {
        // Whether or not /dev/input is readable in the test environment, this
        // must produce a usable guard rather than panicking. The test runner
        // is typically not in the 'input' group, so we expect Disabled.
        let _ = ModifierGuard::new();
    }
}
