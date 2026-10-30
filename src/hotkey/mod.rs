//! Hotkey detection module
//!
//! Provides kernel-level key event detection using evdev.
//! This approach works on all Wayland compositors because it
//! operates at the Linux input subsystem level.
//!
//! Requires the user to be in the 'input' group.

pub mod evdev_listener;

use crate::config::HotkeyConfig;
use crate::error::HotkeyError;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Events emitted by the hotkey listener
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The hotkey was pressed, optionally with a model override and/or profile override
    Pressed {
        /// Model to use for this transcription (None = use default)
        model_override: Option<String>,
        /// Profile to activate for post-processing (None = use default)
        profile_override: Option<String>,
    },
    /// The hotkey was released
    Released,
    /// The cancel key was pressed (abort recording/transcription)
    Cancel,
    /// The correction key has been held cleanly past its threshold while
    /// recording. The correction began when the key went down, at
    /// `pressed_at`, so speech from that moment on belongs to it.
    CorrectionStart { pressed_at: Instant },
    /// The correction key was let go, or a hotkey key was pressed during the
    /// hold: the correction ends here and should be applied.
    CorrectionEnd,
    /// Another key was pressed during the hold (a typed capital, a
    /// shortcut): drop the correction and keep its words as dictation.
    CorrectionCancel,
    /// A clean single tap of the edit key ended. Exactly one
    /// `EditTapConfirmed` or `EditTapCancelled` follows.
    EditTapPending,
    /// The pending tap stayed single for the double-tap window: it asks
    /// for the keyboard edit.
    EditTapConfirmed,
    /// The pending tap turned into something else (a double tap, a hold,
    /// another key) or the recording ended.
    EditTapCancelled,
}

/// Trait for hotkey detection implementations
pub trait HotkeyListener: Send {
    /// Start listening for hotkey events
    /// Returns a channel receiver for events
    fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, HotkeyError>;

    /// Stop listening and clean up
    fn stop(&mut self) -> Result<(), HotkeyError>;

    /// A flag the daemon sets while it records. Correction holds are only
    /// watched while it is set. None when no correction key is configured.
    fn correction_armed(&self) -> Option<Arc<AtomicBool>> {
        None
    }

    /// A flag the daemon sets while a single edit-key tap means something
    /// (recording with keyboard editing on). None when no edit key is set.
    fn edit_armed(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

/// Factory function to create the appropriate hotkey listener.
/// `keyboard_edit`: watch for single taps of the edit key.
pub fn create_listener(
    config: &HotkeyConfig,
    secondary_model: Option<String>,
    keyboard_edit: bool,
) -> Result<Box<dyn HotkeyListener>, HotkeyError> {
    let mut listener = evdev_listener::EvdevListener::new(config)?;
    listener.set_secondary_model(secondary_model);
    if let Some(key) = config.edit_key().filter(|_| keyboard_edit) {
        listener.set_edit_tap(key, Duration::from_millis(config.edit_quiet_ms))?;
    }
    Ok(Box::new(listener))
}
