//! Hotkey settings: PTT key, mode, cancel/modifier keys, double tap,
//! correction and edit keys, look chord, evdev enable.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::app::{Action, App};
use super::common::{
    self, FeedbackLevel as CommonFeedback, FormRowSpec, TextInput, TextInputResult,
};
use super::compositor_bindings;
use super::config_editor::{ConfigEditor, EditorError};

/// In-memory copy of the hotkey state, owned by `App`. Edits mutate this; `s`
/// commits via [`ConfigEditor`] and rolls back on validation error.
#[derive(Debug, Clone)]
pub struct HotkeyState {
    pub key: String,
    pub mode: Mode,
    pub enabled: bool,
    pub cancel_key: Option<String>,
    pub modifier: Option<String>,
    pub double_tap: bool,
    pub double_tap_ms: i64,
    pub double_tap_alt_key: Option<String>,
    pub double_tap_cancel_key: Option<String>,
    pub look_chord: bool,
    pub correction_key: Option<String>,
    pub correction_hold_ms: i64,
    pub edit_key: Option<String>,
    pub edit_quiet_ms: i64,
    /// Status banner shown after Save / Reset, cleared on the next edit.
    pub feedback: Option<Feedback>,
    pub dirty_since_load: bool,
    pub field: Field,
    pub editing: Option<TextEdit>,
}

#[derive(Debug, Clone)]
pub struct TextEdit {
    pub field: Field,
    pub input: TextInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub level: FeedbackLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum FeedbackLevel {
    Ok,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Enabled,
    Key,
    Mode,
    CancelKey,
    Modifier,
    DoubleTap,
    DoubleTapMs,
    DoubleTapAltKey,
    DoubleTapCancelKey,
    LookChord,
    CorrectionKey,
    CorrectionHoldMs,
    EditKey,
    EditQuietMs,
}

impl Field {
    const ALL: &'static [Field] = &[
        Field::Enabled,
        Field::Key,
        Field::Mode,
        Field::CancelKey,
        Field::Modifier,
        Field::DoubleTap,
        Field::DoubleTapMs,
        Field::DoubleTapAltKey,
        Field::DoubleTapCancelKey,
        Field::LookChord,
        Field::CorrectionKey,
        Field::CorrectionHoldMs,
        Field::EditKey,
        Field::EditQuietMs,
    ];
}

/// Sensible PTT key choices, in order. Values match what voxtype's evdev
/// listener accepts (KEY_* names without the prefix).
const KEY_CHOICES: &[&str] = &[
    "HOME",
    "PAUSE",
    "SCROLLLOCK",
    "INSERT",
    "MENU",
    "F13",
    "F14",
    "F15",
    "F16",
    "RIGHTCTRL",
    "RIGHTALT",
    "RIGHTMETA",
    "CAPSLOCK",
];

const CANCEL_CHOICES: &[Option<&str>] = &[
    None,
    Some("ESC"),
    Some("BACKSPACE"),
    Some("F12"),
    Some("DELETE"),
    Some("END"),
];

const MODIFIER_CHOICES: &[Option<&str>] = &[
    None,
    Some("LEFTSHIFT"),
    Some("RIGHTSHIFT"),
    Some("LEFTCTRL"),
    Some("LEFTALT"),
    Some("LEFTMETA"),
];

/// Keys for the second double-tap, correction and edit keys. Modifiers suit
/// them because a tap or a hold of one alone types nothing.
const SIDE_KEY_CHOICES: &[Option<&str>] = &[
    None,
    Some("RIGHTSHIFT"),
    Some("RIGHTALT"),
    Some("RIGHTCTRL"),
    Some("RIGHTMETA"),
    Some("LEFTSHIFT"),
    Some("LEFTCTRL"),
    Some("LEFTALT"),
    Some("LEFTMETA"),
];

// Defaults match `HotkeyConfig`; a key at its default stays out of the file.
const DEFAULT_DOUBLE_TAP_MS: i64 = 300;
const DEFAULT_CORRECTION_HOLD_MS: i64 = 450;
const DEFAULT_EDIT_QUIET_MS: i64 = 150;
const DOUBLE_TAP_MS_CHOICES: &[i64] = &[200, 250, 300, 350, 400, 500, 600];
const CORRECTION_HOLD_CHOICES: &[i64] = &[300, 400, 450, 500, 600, 800, 1000];
const EDIT_QUIET_CHOICES: &[i64] = &[0, 100, 150, 200, 300, 500];

impl HotkeyState {
    pub fn load() -> Result<Self, EditorError> {
        let ed = ConfigEditor::load()?;
        Ok(Self {
            key: ed
                .get_string("hotkey", "key")
                .unwrap_or_else(|| "HOME".to_string()),
            mode: match ed.get_string("hotkey", "mode").as_deref() {
                Some("toggle") => Mode::Toggle,
                _ => Mode::PushToTalk,
            },
            enabled: ed.get_bool("hotkey", "enabled").unwrap_or(true),
            cancel_key: ed.get_string("hotkey", "cancel_key"),
            modifier: ed.get_string("hotkey", "model_modifier"),
            double_tap: ed.get_bool("hotkey", "double_tap").unwrap_or(false),
            double_tap_ms: ed
                .get_int("hotkey", "double_tap_ms")
                .unwrap_or(DEFAULT_DOUBLE_TAP_MS),
            double_tap_alt_key: ed.get_string("hotkey", "double_tap_alt_key"),
            double_tap_cancel_key: ed.get_string("hotkey", "double_tap_cancel_key"),
            look_chord: ed.get_bool("hotkey", "look_chord").unwrap_or(true),
            correction_key: ed.get_string("hotkey", "correction_key"),
            correction_hold_ms: ed
                .get_int("hotkey", "correction_hold_ms")
                .unwrap_or(DEFAULT_CORRECTION_HOLD_MS),
            edit_key: ed.get_string("hotkey", "edit_key"),
            edit_quiet_ms: ed
                .get_int("hotkey", "edit_quiet_ms")
                .unwrap_or(DEFAULT_EDIT_QUIET_MS),
            feedback: None,
            dirty_since_load: false,
            field: Field::Enabled,
            editing: None,
        })
    }

    pub fn save(&mut self) -> Action {
        let mut ed = match ConfigEditor::load() {
            Ok(e) => e,
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("load: {}", e),
                });
                return Action::None;
            }
        };
        ed.set_string("hotkey", "key", &self.key);
        ed.set_string(
            "hotkey",
            "mode",
            match self.mode {
                Mode::PushToTalk => "push_to_talk",
                Mode::Toggle => "toggle",
            },
        );
        ed.set_bool("hotkey", "enabled", self.enabled);
        match &self.cancel_key {
            Some(k) => ed.set_string("hotkey", "cancel_key", k),
            None => ed.unset("hotkey", "cancel_key"),
        }
        match &self.modifier {
            Some(k) => ed.set_string("hotkey", "model_modifier", k),
            None => ed.unset("hotkey", "model_modifier"),
        }
        set_or_unset_bool(&mut ed, "double_tap", self.double_tap, false);
        set_or_unset_int(
            &mut ed,
            "double_tap_ms",
            self.double_tap_ms,
            DEFAULT_DOUBLE_TAP_MS,
        );
        set_or_unset_key(&mut ed, "double_tap_alt_key", &self.double_tap_alt_key);
        set_or_unset_key(
            &mut ed,
            "double_tap_cancel_key",
            &self.double_tap_cancel_key,
        );
        set_or_unset_bool(&mut ed, "look_chord", self.look_chord, true);
        set_or_unset_key(&mut ed, "correction_key", &self.correction_key);
        set_or_unset_int(
            &mut ed,
            "correction_hold_ms",
            self.correction_hold_ms,
            DEFAULT_CORRECTION_HOLD_MS,
        );
        set_or_unset_key(&mut ed, "edit_key", &self.edit_key);
        set_or_unset_int(
            &mut ed,
            "edit_quiet_ms",
            self.edit_quiet_ms,
            DEFAULT_EDIT_QUIET_MS,
        );

        match ed.save() {
            Ok(()) => {
                self.dirty_since_load = false;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: format!("Saved to {}", ed.path().display()),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("save: {}", e),
                });
            }
        }
        Action::None
    }

    pub fn reset(&mut self) {
        match Self::load() {
            Ok(fresh) => {
                let field = self.field;
                *self = fresh;
                self.field = field;
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Ok,
                    message: "Reverted unsaved changes".to_string(),
                });
            }
            Err(e) => {
                self.feedback = Some(Feedback {
                    level: FeedbackLevel::Err,
                    message: format!("reload: {}", e),
                });
            }
        }
    }

    fn move_field(&mut self, delta: i32) {
        let len = Field::ALL.len() as i32;
        let cur = Field::ALL
            .iter()
            .position(|f| *f == self.field)
            .unwrap_or(0) as i32;
        let new = (cur + delta).rem_euclid(len);
        self.field = Field::ALL[new as usize];
    }

    /// Cycle the value of the focused field by `delta` (-1 for ← / +1 for →).
    fn is_text_field(field: Field) -> bool {
        // Free-text on Key / CancelKey / Modifier so users can type custom
        // KEY_* names that aren't in the curated cycle list.
        matches!(
            field,
            Field::Key
                | Field::CancelKey
                | Field::Modifier
                | Field::DoubleTapAltKey
                | Field::DoubleTapCancelKey
                | Field::CorrectionKey
                | Field::EditKey
        )
    }

    /// The optional key a text field edits, or `None` for the other fields.
    fn optional_key_mut(&mut self, field: Field) -> Option<&mut Option<String>> {
        match field {
            Field::CancelKey => Some(&mut self.cancel_key),
            Field::Modifier => Some(&mut self.modifier),
            Field::DoubleTapAltKey => Some(&mut self.double_tap_alt_key),
            Field::DoubleTapCancelKey => Some(&mut self.double_tap_cancel_key),
            Field::CorrectionKey => Some(&mut self.correction_key),
            Field::EditKey => Some(&mut self.edit_key),
            _ => None,
        }
    }

    fn start_edit_if_text_field(&mut self) -> bool {
        // Edit only makes sense when the listener is enabled — otherwise
        // these fields are dimmed/inert.
        if !self.enabled || !Self::is_text_field(self.field) {
            return false;
        }
        let field = self.field;
        let initial = match field {
            Field::Key => self.key.clone(),
            _ => self
                .optional_key_mut(field)
                .and_then(|k| k.clone())
                .unwrap_or_default(),
        };
        self.editing = Some(TextEdit {
            field: self.field,
            input: TextInput::new(initial),
        });
        true
    }

    fn commit_text_edit(&mut self, field: Field, buffer: String) {
        let trimmed = buffer.trim();
        match field {
            Field::Key if !trimmed.is_empty() => {
                self.key = trimmed.to_uppercase();
            }
            _ => {
                if let Some(k) = self.optional_key_mut(field) {
                    *k = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_uppercase())
                    };
                }
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }

    fn cycle(&mut self, delta: i32) {
        // When the evdev listener is off, only the Enabled toggle responds —
        // the rest of the form is greyed out and inert.
        if !self.enabled && self.field != Field::Enabled {
            return;
        }
        match self.field {
            Field::Key => {
                self.key = cycle_str(KEY_CHOICES, &self.key, delta);
            }
            Field::Mode => {
                self.mode = match self.mode {
                    Mode::PushToTalk => Mode::Toggle,
                    Mode::Toggle => Mode::PushToTalk,
                };
            }
            Field::CancelKey => {
                self.cancel_key = cycle_opt(CANCEL_CHOICES, self.cancel_key.as_deref(), delta);
            }
            Field::Modifier => {
                self.modifier = cycle_opt(MODIFIER_CHOICES, self.modifier.as_deref(), delta);
            }
            Field::Enabled => {
                self.enabled = !self.enabled;
            }
            Field::DoubleTap => self.double_tap = !self.double_tap,
            Field::DoubleTapMs => {
                self.double_tap_ms = cycle_int(DOUBLE_TAP_MS_CHOICES, self.double_tap_ms, delta)
            }
            Field::DoubleTapAltKey => {
                self.double_tap_alt_key =
                    cycle_opt(SIDE_KEY_CHOICES, self.double_tap_alt_key.as_deref(), delta)
            }
            Field::DoubleTapCancelKey => {
                self.double_tap_cancel_key =
                    cycle_opt(CANCEL_CHOICES, self.double_tap_cancel_key.as_deref(), delta)
            }
            Field::LookChord => self.look_chord = !self.look_chord,
            Field::CorrectionKey => {
                self.correction_key =
                    cycle_opt(SIDE_KEY_CHOICES, self.correction_key.as_deref(), delta)
            }
            Field::CorrectionHoldMs => {
                self.correction_hold_ms =
                    cycle_int(CORRECTION_HOLD_CHOICES, self.correction_hold_ms, delta)
            }
            Field::EditKey => {
                self.edit_key = cycle_opt(SIDE_KEY_CHOICES, self.edit_key.as_deref(), delta)
            }
            Field::EditQuietMs => {
                self.edit_quiet_ms = cycle_int(EDIT_QUIET_CHOICES, self.edit_quiet_ms, delta)
            }
        }
        self.dirty_since_load = true;
        self.feedback = None;
    }
}

fn cycle_str(choices: &[&'static str], current: &str, delta: i32) -> String {
    if choices.is_empty() {
        return current.to_string();
    }
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize].to_string()
}

fn cycle_int(choices: &[i64], current: i64, delta: i32) -> i64 {
    let idx = choices
        .iter()
        .position(|c| *c == current)
        .map(|i| i as i32)
        .unwrap_or(-1);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize]
}

fn set_or_unset_bool(ed: &mut ConfigEditor, key: &str, value: bool, default: bool) {
    if value == default {
        ed.unset("hotkey", key);
    } else {
        ed.set_bool("hotkey", key, value);
    }
}

fn set_or_unset_int(ed: &mut ConfigEditor, key: &str, value: i64, default: i64) {
    if value == default {
        ed.unset("hotkey", key);
    } else {
        ed.set_int("hotkey", key, value);
    }
}

fn set_or_unset_key(ed: &mut ConfigEditor, key: &str, value: &Option<String>) {
    match value {
        Some(k) => ed.set_string("hotkey", key, k),
        None => ed.unset("hotkey", key),
    }
}

fn cycle_opt(
    choices: &[Option<&'static str>],
    current: Option<&str>,
    delta: i32,
) -> Option<String> {
    if choices.is_empty() {
        return current.map(|s| s.to_string());
    }
    let idx = choices
        .iter()
        .position(|c| c.as_deref() == current)
        .map(|i| i as i32)
        .unwrap_or(0);
    let new = (idx + delta).rem_euclid(choices.len() as i32);
    choices[new as usize].map(|s| s.to_string())
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let state = match &app.hotkey {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Hotkey");
            let inner = block.inner(area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new("Failed to load config; check ~/.config/voxtype/config.toml.")
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        }
    };

    // Greyout fields after Enabled when the evdev listener is off — those
    // controls don't affect anything until the listener turns back on.
    let greyout = !state.enabled;

    let rows = vec![
        FormRowSpec::new(
            state.field == Field::Enabled,
            "Built-in evdev listener",
            if state.enabled { "enabled" } else { "disabled" },
        ),
        FormRowSpec::new(
            state.field == Field::Key,
            "Push-to-talk key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Key => e.input.caret_string(),
                _ => display_key(&state.key),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Mode,
            "Mode",
            match state.mode {
                Mode::PushToTalk => "Push-to-talk (hold)",
                Mode::Toggle => "Toggle (press to start/stop)",
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::CancelKey,
            "Cancel key",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::CancelKey => e.input.caret_string(),
                _ => state.cancel_key.as_deref().unwrap_or("(none)").to_string(),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::Modifier,
            "Modifier (secondary model)",
            match state.editing.as_ref() {
                Some(e) if e.field == Field::Modifier => e.input.caret_string(),
                _ => state.modifier.as_deref().unwrap_or("(none)").to_string(),
            },
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::DoubleTap,
            "Double tap",
            yesno(state.double_tap),
        )
        .dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::DoubleTapMs,
            "Double tap time (ms)",
            state.double_tap_ms.to_string(),
        )
        .dimmed(greyout),
        key_row(state, Field::DoubleTapAltKey, "Second double-tap key").dimmed(greyout),
        key_row(state, Field::DoubleTapCancelKey, "Double-tap cancel key").dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::LookChord,
            "Look chord (Ctrl+Ctrl+Up/Down)",
            yesno(state.look_chord),
        )
        .dimmed(greyout),
        key_row(state, Field::CorrectionKey, "Correction key").dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::CorrectionHoldMs,
            "Correction hold (ms)",
            state.correction_hold_ms.to_string(),
        )
        .dimmed(greyout),
        key_row(state, Field::EditKey, "Edit key").dimmed(greyout),
        FormRowSpec::new(
            state.field == Field::EditQuietMs,
            "Edit tap quiet time (ms)",
            state.edit_quiet_ms.to_string(),
        )
        .dimmed(greyout),
    ];

    let feedback_pair = state
        .feedback
        .as_ref()
        .map(|fb| (to_common_level(fb.level), fb.message.as_str()));

    let guidance = guidance_for_field(state);

    common::render_form_with_guidance(
        f,
        area,
        "Hotkey",
        state.dirty_since_load,
        feedback_pair,
        &rows,
        guidance,
    );
}

fn yesno(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// Row for an optional key: the inline editor while editing, else the key.
fn key_row(state: &HotkeyState, field: Field, label: &'static str) -> FormRowSpec {
    let value = match state.editing.as_ref() {
        Some(e) if e.field == field => e.input.caret_string(),
        _ => {
            let key = match field {
                Field::DoubleTapAltKey => &state.double_tap_alt_key,
                Field::DoubleTapCancelKey => &state.double_tap_cancel_key,
                Field::CorrectionKey => &state.correction_key,
                Field::EditKey => &state.edit_key,
                _ => &None,
            };
            key.as_deref().unwrap_or("(none)").to_string()
        }
    };
    FormRowSpec::new(state.field == field, label, value)
}

fn to_common_level(level: FeedbackLevel) -> CommonFeedback {
    match level {
        FeedbackLevel::Ok => CommonFeedback::Ok,
        FeedbackLevel::Err => CommonFeedback::Err,
    }
}

/// Right-pane explanation for the focused field.
fn guidance_for_field(state: &HotkeyState) -> Vec<Line<'_>> {
    match state.field {
        Field::Enabled => guidance_enabled(state),
        Field::Key => guidance_key(state),
        Field::Mode => guidance_mode(state),
        Field::CancelKey => guidance_cancel(state),
        Field::Modifier => guidance_modifier(state),
        Field::DoubleTap => simple_guidance(
            state,
            "Double tap",
            "Start and stop recording with a quick double tap of the push-to-talk \
             key instead of one press. A tap is a press and release with no \
             other key pressed meanwhile, so shortcuts that use the key (Ctrl+C \
             on Right Ctrl) never count. Use it with Toggle mode.",
            "Off by default.",
        ),
        Field::DoubleTapMs => simple_guidance(
            state,
            "Double tap time",
            "The longest tap, and the longest gap between the two taps, in \
             milliseconds.",
            "Default: 300.",
        ),
        Field::DoubleTapAltKey => simple_guidance(
            state,
            "Second double-tap key",
            "A second key whose double tap also starts and stops recording (with \
             Double tap on). Each double tap writes which key fired, main or alt, \
             to hotkey-last in the runtime directory, so an output hook can act \
             on how the recording was stopped.",
            "(none) by default. Enter or i types any KEY_* name.",
        ),
        Field::DoubleTapCancelKey => simple_guidance(
            state,
            "Double-tap cancel key",
            "A key whose double tap discards the recording (with Double tap on). \
             Unlike the single-press cancel key, a stray press never throws a \
             dictation away.",
            "(none) by default. Enter or i types any KEY_* name.",
        ),
        Field::LookChord => simple_guidance(
            state,
            "Look chord",
            "Hold Left Ctrl and Right Ctrl, then press Down or Up, to switch the \
             vo popup to its next or previous look while it shows. The look is \
             kept for later dictations.",
            "On by default.",
        ),
        Field::CorrectionKey => simple_guidance(
            state,
            "Correction key",
            "Hold this key while recording and say the right words; they replace \
             the words they sound most like. Taps keep their meaning, so it can \
             be the second double-tap key. Needs a live preview model.",
            "(none) by default, which turns corrections off.",
        ),
        Field::CorrectionHoldMs => simple_guidance(
            state,
            "Correction hold",
            "How long the correction key must be held before a correction \
             starts, in milliseconds. With Double tap on it is raised to at \
             least the double tap time plus 150.",
            "Default: 450.",
        ),
        Field::EditKey => simple_guidance(
            state,
            "Edit key",
            "A single tap while recording pauses the dictation so you can fix \
             its text by keyboard in the popup; tap again to keep talking. \
             Needs [osd] keyboard_edit.",
            "(none) by default, which uses the second double-tap key.",
        ),
        Field::EditQuietMs => simple_guidance(
            state,
            "Edit tap quiet time",
            "The edit tap counts only when no other key was pressed this many \
             milliseconds before it, so a Shift pressed while typing does not \
             pause the dictation.",
            "Default: 150.",
        ),
    }
}

fn heading<'a>(text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn guidance_enabled<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Built-in evdev listener"),
        Line::from(""),
        Line::from(
            "When enabled, voxtype reads keyboard events directly from \
             /dev/input/event* (your user must be in the `input` group). It \
             owns the chosen PTT key globally — no compositor binding needed.",
        ),
        Line::from(""),
        Line::from(
            "When disabled, voxtype reads no keys. Bind your compositor (\
             Hyprland, Sway, Niri, KDE shortcuts) to call:",
        ),
        Line::from(Span::styled(
            "    voxtype record start    voxtype record stop",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            "    voxtype record toggle   voxtype record cancel",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ];

    let bindings = compositor_bindings::detect();
    if !bindings.is_empty() {
        lines.push(Line::from(Span::styled(
            "Compositor bindings detected",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for b in &bindings {
            let file = b.source.file_name().and_then(|s| s.to_str()).unwrap_or("");
            lines.push(Line::from(format!(
                "  • [{}] {}  →  voxtype {}",
                b.compositor, b.keys, b.action
            )));
            lines.push(Line::from(Span::styled(
                format!("      from {}", file),
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(""));
    } else if !state.enabled {
        lines.push(Line::from(Span::styled(
            "No compositor bindings detected — voxtype will not receive any \
             PTT key events.",
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }

    // Streaming dictation requires toggle activation; if the user has it
    // enabled, suppress PTT-pair suggestions in favor of a toggle binding.
    // Covers all streaming-capable backends (Parakeet, Soniox, future).
    let streaming = {
        let ed = ConfigEditor::load().ok();
        let engine = ed
            .as_ref()
            .and_then(|e| e.get_string("", "engine"))
            .unwrap_or_else(|| "whisper".to_string());
        match engine.as_str() {
            "parakeet" => ed
                .as_ref()
                .and_then(|e| e.get_bool("parakeet", "streaming"))
                .unwrap_or(false),
            "soniox" => ed
                .as_ref()
                .and_then(|e| e.get_bool("soniox", "streaming"))
                .unwrap_or(true),
            _ => false,
        }
    };
    let suggestions = compositor_bindings::suggest_missing(&bindings, streaming);
    if !suggestions.is_empty() {
        let comp = compositor_bindings::dominant_compositor(&bindings);
        lines.push(Line::from(Span::styled(
            format!("Suggested additions ({} format)", comp.name()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for s in &suggestions {
            lines.push(Line::from(Span::styled(
                format!("  ▸ {}", s.label),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(format!("    {}", s.purpose)));
            for cfg in &s.config_lines {
                lines.push(Line::from(Span::styled(
                    format!("    {}", cfg),
                    Style::default().fg(Color::Gray),
                )));
            }
            lines.push(Line::from(""));
        }
    }

    if !state.enabled {
        lines.push(Line::from(Span::styled(
            "Compositor mode active: the rest of this section is ignored.",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_key<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Push-to-talk key"),
        Line::from(""),
        Line::from(
            "Pick a key your fingers reach for without thinking. HOME, PAUSE, \
             SCROLLLOCK, F13 are popular because they don't conflict with \
             editor shortcuts.",
        ),
        Line::from(""),
        Line::from(
            "RIGHT* keys (RIGHTCTRL, RIGHTALT, RIGHTMETA) work well if you \
             touch-type with your left hand on the home row.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Custom keys can be set in config.toml directly using KEY_* \
             names from <linux/input-event-codes.h>.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_mode<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Activation mode"),
        Line::from(""),
        Line::from(Span::styled(
            "Push-to-talk: ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Hold the key while you speak; release to transcribe. Most \
             responsive — voice never starts running while you're thinking.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Toggle: ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Press once to start, press again to stop. Friendlier for long \
             dictation sessions but easy to leave running by accident.",
        ),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_cancel<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Cancel key"),
        Line::from(""),
        Line::from(
            "Aborts an in-progress recording or transcription and discards \
             audio without typing anything. Useful when you trip the PTT key \
             by accident or the wrong window is focused.",
        ),
        Line::from(""),
        Line::from(
            "ESC is the obvious pick. F12 / DELETE / END are good alternatives \
             if ESC is bound to something else in the foreground app.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "(none) leaves cancellation off — kill the recording with \
             `voxtype record cancel` instead.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn guidance_modifier<'a>(state: &'a HotkeyState) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading("Secondary-model modifier"),
        Line::from(""),
        Line::from(
            "When this key is held alongside the PTT key, voxtype switches to \
             the [whisper] secondary_model for that recording.",
        ),
        Line::from(""),
        Line::from(
            "Common usage: large-v3 as your main model for accuracy, \
             small.en under the modifier for instant short notes.",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "(none) disables the modifier behavior; the PTT key always uses \
             the primary model.",
            Style::default().fg(Color::Gray),
        )),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn simple_guidance<'a>(
    state: &'a HotkeyState,
    title: &'a str,
    body: &'a str,
    default: &'a str,
) -> Vec<Line<'a>> {
    let mut lines = vec![
        heading(title),
        Line::from(""),
        Line::from(body),
        Line::from(""),
        Line::from(Span::styled(default, Style::default().fg(Color::Gray))),
    ];
    if !state.enabled {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(Ignored: evdev listener is disabled.)",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines
}

fn display_key(key: &str) -> String {
    if KEY_CHOICES.contains(&key) {
        key.to_string()
    } else {
        format!("{}  (custom)", key)
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let state = match app.hotkey.as_mut() {
        Some(s) => s,
        None => return Action::None,
    };

    if state.editing.is_some() {
        return handle_edit_key(state, key);
    }

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.move_field(-1);
            Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.move_field(1);
            Action::None
        }
        KeyCode::Left | KeyCode::Char('h') => {
            state.cycle(-1);
            Action::None
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
            state.cycle(1);
            Action::None
        }
        KeyCode::Enter | KeyCode::Char('i') => {
            state.start_edit_if_text_field();
            Action::None
        }
        KeyCode::Char('s') => state.save(),
        KeyCode::Char('r') => {
            state.reset();
            Action::None
        }
        _ => Action::None,
    }
}

fn handle_edit_key(state: &mut HotkeyState, key: KeyEvent) -> Action {
    let Some(editing) = state.editing.as_mut() else {
        return Action::None;
    };
    match editing.input.handle_key(key) {
        TextInputResult::Continue => Action::None,
        TextInputResult::Commit => {
            let buf = editing.input.buffer().to_string();
            let field = editing.field;
            state.editing = None;
            state.commit_text_edit(field, buf);
            Action::None
        }
        TextInputResult::Cancel => {
            state.editing = None;
            Action::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{scalar_keys, CONFIG_KEYS};

    /// The hotkey form is a hand-written transcription of the schema's
    /// `Hotkey` section, so a key added in one place and not the other
    /// drifts silently. If a key genuinely does not belong in the TUI, add
    /// it to `NOT_IN_TUI` with a reason rather than deleting the test.
    #[test]
    fn every_hotkey_schema_key_has_a_form_field() {
        const NOT_IN_TUI: &[&str] = &[];

        let expected: Vec<&str> = scalar_keys()
            .filter(|s| s.section == "Hotkey")
            .map(|s| s.key)
            .filter(|k| !NOT_IN_TUI.contains(k))
            .collect();

        // Field -> the schema key it edits. Adding a Field without extending
        // this map fails to compile, which is the point.
        let covered: Vec<&str> = Field::ALL
            .iter()
            .map(|f| match f {
                Field::Enabled => "hotkey.enabled",
                Field::Key => "hotkey.key",
                Field::Mode => "hotkey.mode",
                Field::CancelKey => "hotkey.cancel_key",
                Field::Modifier => "hotkey.model_modifier",
                Field::DoubleTap => "hotkey.double_tap",
                Field::DoubleTapMs => "hotkey.double_tap_ms",
                Field::DoubleTapAltKey => "hotkey.double_tap_alt_key",
                Field::DoubleTapCancelKey => "hotkey.double_tap_cancel_key",
                Field::LookChord => "hotkey.look_chord",
                Field::CorrectionKey => "hotkey.correction_key",
                Field::CorrectionHoldMs => "hotkey.correction_hold_ms",
                Field::EditKey => "hotkey.edit_key",
                Field::EditQuietMs => "hotkey.edit_quiet_ms",
            })
            .collect();

        let missing: Vec<&&str> = expected.iter().filter(|k| !covered.contains(k)).collect();
        assert!(
            missing.is_empty(),
            "schema keys in section Hotkey with no TUI field: {missing:?}"
        );

        let unknown: Vec<&&str> = covered
            .iter()
            .filter(|k| !CONFIG_KEYS.iter().any(|s| s.key == **k))
            .collect();
        assert!(
            unknown.is_empty(),
            "TUI fields editing keys absent from the schema: {unknown:?}"
        );
    }

    fn state() -> HotkeyState {
        HotkeyState {
            key: "RIGHTCTRL".to_string(),
            mode: Mode::Toggle,
            enabled: true,
            cancel_key: None,
            modifier: None,
            double_tap: false,
            double_tap_ms: DEFAULT_DOUBLE_TAP_MS,
            double_tap_alt_key: None,
            double_tap_cancel_key: None,
            look_chord: true,
            correction_key: None,
            correction_hold_ms: DEFAULT_CORRECTION_HOLD_MS,
            edit_key: None,
            edit_quiet_ms: DEFAULT_EDIT_QUIET_MS,
            feedback: None,
            dirty_since_load: false,
            field: Field::Enabled,
            editing: None,
        }
    }

    #[test]
    fn optional_key_fields_cycle_and_take_typed_names() {
        let mut s = state();
        s.field = Field::CorrectionKey;
        s.cycle(1);
        assert_eq!(s.correction_key.as_deref(), Some("RIGHTSHIFT"));
        s.cycle(-1);
        assert_eq!(s.correction_key, None);

        s.commit_text_edit(Field::EditKey, " rightalt ".to_string());
        assert_eq!(s.edit_key.as_deref(), Some("RIGHTALT"));
        s.commit_text_edit(Field::EditKey, "  ".to_string());
        assert_eq!(s.edit_key, None);
    }

    #[test]
    fn timing_fields_cycle_from_their_defaults() {
        let mut s = state();
        s.field = Field::DoubleTapMs;
        s.cycle(1);
        assert_eq!(s.double_tap_ms, 350);
        s.field = Field::CorrectionHoldMs;
        s.cycle(-1);
        assert_eq!(s.correction_hold_ms, 400);
        s.field = Field::EditQuietMs;
        s.cycle(1);
        assert_eq!(s.edit_quiet_ms, 200);
    }
}
