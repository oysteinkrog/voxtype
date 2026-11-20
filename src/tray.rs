//! Linux StatusNotifierItem and dbusmenu integration, adapted from PR #438.
//! The daemon owns one state channel. ksni owns registration and watcher recovery.
//! Nothing here opens an audio device or installs files at daemon startup.
//!
//! Ported from upstream voxtype v1.3.1's `src/tray.rs` and trimmed to the
//! dictation entries: Start/Stop recording, Cancel, the recent dictations
//! and Copy last dictation. Upstream's meeting, model upgrade, language,
//! settings and history window entries are left out, and the icon comes
//! from the icon theme instead of bundled pixels. The menu's content is
//! worked out by the pure functions [`menu_items`], [`recent_dictations`],
//! [`label`] and [`click_text`]; the ksni side only maps them.

use ksni::{menu::*, Status, ToolTip, Tray, TrayMethods};
use std::{sync::Arc, time::Duration};
use tokio::{
    process::Command,
    sync::{mpsc, watch},
};

use crate::history::{recorder::Recorder, HistoryEntry, HistoryStore};
use crate::last_dictation::LastDictation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Transcribing,
    Error,
    Stopped,
}

impl TrayState {
    pub fn from_state_name(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "idle" => Self::Idle,
            "recording" | "streaming" => Self::Recording,
            "transcribing" | "outputting" | "loading" => Self::Transcribing,
            "error" => Self::Error,
            "stopped" => Self::Stopped,
            other => {
                tracing::debug!(state = other, "Unknown tray state; showing ready");
                Self::Idle
            }
        }
    }
    fn sni_status(self) -> Status {
        match self {
            Self::Recording | Self::Error => Status::NeedsAttention,
            Self::Stopped => Status::Passive,
            _ => Status::Active,
        }
    }
    fn tooltip_body(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Recording => "Recording...",
            Self::Transcribing => "Transcribing...",
            Self::Error => "Dictation failed. Check the Voxtype logs; you can try recording again.",
            Self::Stopped => "Daemon not running",
        }
    }
    /// A freedesktop icon name for the state. KDE Plasma and GNOME's
    /// AppIndicator extension both resolve these from the icon theme.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Idle => "audio-input-microphone",
            Self::Recording => "media-record",
            Self::Transcribing => "view-refresh",
            Self::Error => "dialog-error",
            Self::Stopped => "microphone-sensitivity-muted",
        }
    }
}

/// One recent dictation the menu can list.
#[derive(Debug, Clone, PartialEq)]
pub struct Recent {
    /// The history entry ID, or `None` for `last-dictation.json`.
    pub id: Option<String>,
    pub text: String,
}

/// The dictations the menu lists, newest first, at most `limit`. From
/// history while it records; otherwise the one dictation kept in
/// `last-dictation.json`, which is then the newest. Entries without text
/// are skipped: there is nothing to copy.
pub fn recent_dictations(
    history_recording: bool,
    mut entries: Vec<HistoryEntry>,
    kept: Option<LastDictation>,
    limit: usize,
) -> Vec<Recent> {
    entries.retain(|e| !e.best_text().trim().is_empty());
    if history_recording && !entries.is_empty() {
        entries.sort_by_key(|e| std::cmp::Reverse(e.ts_start));
        return entries
            .into_iter()
            .take(limit)
            .map(|e| Recent {
                text: e.best_text().to_string(),
                id: Some(e.id),
            })
            .collect();
    }
    kept.filter(|k| !k.text.trim().is_empty())
        .map(|k| Recent {
            id: None,
            text: k.text,
        })
        .into_iter()
        .take(limit)
        .collect()
}

/// How many characters of a dictation its menu label shows.
pub const LABEL_CHARS: usize = 40;

/// A dictation's menu label: its first [`LABEL_CHARS`] characters on one
/// line (control characters become spaces), with "…" when cut.
pub fn label(text: &str) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.trim();
    if line.chars().count() <= LABEL_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(LABEL_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// The text a click on the `index`th Recent entry (0 is the newest) copies.
pub fn click_text(recent: &[Recent], index: usize) -> Option<&str> {
    recent.get(index).map(|r| r.text.as_str())
}

/// One menu entry, before it becomes a ksni item.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// The disabled first line: "Voxtype: Ready".
    Status(String),
    /// Start Recording, or Stop Recording while recording.
    Toggle {
        stop: bool,
        enabled: bool,
    },
    Cancel {
        enabled: bool,
    },
    /// The Recent submenu: one label per dictation, newest first.
    Recent(Vec<String>),
    CopyLast {
        enabled: bool,
    },
    Separator,
}

/// The menu for `state`, listing up to `shown` of `recent` (newest first).
/// `shown` 0 leaves the Recent submenu out.
pub fn menu_items(state: TrayState, recent: &[Recent], shown: usize) -> Vec<Item> {
    let mut items = vec![
        Item::Status(format!("Voxtype: {}", state.tooltip_body())),
        Item::Separator,
        Item::Toggle {
            stop: state == TrayState::Recording,
            enabled: toggle_allowed(state),
        },
        Item::Cancel {
            enabled: cancel_allowed(state),
        },
        Item::Separator,
    ];
    if shown > 0 {
        items.push(Item::Recent(
            recent.iter().take(shown).map(|r| label(&r.text)).collect(),
        ));
    }
    items.push(Item::CopyLast {
        enabled: click_text(recent, 0).is_some(),
    });
    items
}

fn toggle_allowed(state: TrayState) -> bool {
    !matches!(state, TrayState::Transcribing | TrayState::Stopped)
}

fn cancel_allowed(state: TrayState) -> bool {
    matches!(state, TrayState::Recording | TrayState::Transcribing)
}

/// Closed set of actions. No user-controlled command strings or shell evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    ToggleRecording,
    Cancel,
    /// Put this text on the clipboard.
    Copy(String),
}

impl TrayAction {
    /// The `voxtype` arguments an action runs, for the actions that are
    /// plain CLI commands.
    pub fn cli_args(&self) -> Option<&'static [&'static str]> {
        Some(match self {
            Self::ToggleRecording => &["record", "toggle"],
            Self::Cancel => &["record", "cancel"],
            Self::Copy(_) => return None,
        })
    }
}

type Actions = Arc<dyn Fn(TrayAction) + Send + Sync>;

struct VoxtypeTray {
    state: TrayState,
    /// Newest first; at least the newest one even when `shown` is 0, for
    /// Copy last dictation.
    recent: Vec<Recent>,
    /// `[tray] recent`.
    shown: usize,
    actions: Actions,
    /// Asks the tray task to read the recent dictations again.
    refresh: mpsc::UnboundedSender<()>,
}

/// ksni reads `_` as an access key marker; `__` shows one underscore.
fn escape(label: &str) -> String {
    label.replace('_', "__")
}

impl Tray for VoxtypeTray {
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        "voxtype".into()
    }
    fn title(&self) -> String {
        "Voxtype".into()
    }
    fn icon_name(&self) -> String {
        self.state.icon_name().into()
    }
    fn attention_icon_name(&self) -> String {
        self.state.icon_name().into()
    }
    fn status(&self) -> Status {
        self.state.sni_status()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: format!("Voxtype: {}", self.state.tooltip_body()),
            description: self.state.tooltip_body().into(),
            ..Default::default()
        }
    }
    // Hosts use ItemIsMenu to show the native menu. Legacy Activate is harmless.
    fn activate(&mut self, _x: i32, _y: i32) {}
    fn menu_about_to_show(&mut self) {
        let _ = self.refresh.send(());
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        menu_items(self.state, &self.recent, self.shown)
            .into_iter()
            .map(|item| self.ksni_item(item))
            .collect()
    }
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        tracing::info!(
            ?reason,
            "No tray host; the tray icon appears when one starts"
        );
        true
    }
}

impl VoxtypeTray {
    fn ksni_item(&self, item: Item) -> ksni::MenuItem<Self> {
        match item {
            Item::Status(text) => StandardItem {
                label: escape(&text),
                enabled: false,
                ..Default::default()
            }
            .into(),
            Item::Separator => MenuItem::Separator,
            Item::Toggle { stop, enabled } => StandardItem {
                label: if stop {
                    "Stop Recording"
                } else {
                    "Start Recording"
                }
                .into(),
                enabled,
                activate: Box::new(|this: &mut Self| {
                    // Re-check: the menu may be stale by the time it is clicked.
                    if toggle_allowed(this.state) {
                        (this.actions)(TrayAction::ToggleRecording);
                    }
                }),
                ..Default::default()
            }
            .into(),
            Item::Cancel { enabled } => StandardItem {
                label: "Cancel".into(),
                enabled,
                activate: Box::new(|this: &mut Self| {
                    if cancel_allowed(this.state) {
                        (this.actions)(TrayAction::Cancel);
                    }
                }),
                ..Default::default()
            }
            .into(),
            Item::Recent(labels) => {
                let children: Vec<ksni::MenuItem<Self>> = labels
                    .iter()
                    .enumerate()
                    .map(|(k, l)| {
                        // The text as the menu was built, so a click copies
                        // the entry it shows even if the list moved since.
                        let text = click_text(&self.recent, k).map(str::to_owned);
                        StandardItem {
                            label: escape(l),
                            activate: Box::new(move |this: &mut Self| {
                                if let Some(text) = &text {
                                    (this.actions)(TrayAction::Copy(text.clone()));
                                }
                            }),
                            ..Default::default()
                        }
                        .into()
                    })
                    .collect();
                SubMenu {
                    label: "Recent".into(),
                    enabled: !children.is_empty(),
                    submenu: children,
                    ..Default::default()
                }
                .into()
            }
            Item::CopyLast { enabled } => StandardItem {
                label: "Copy last dictation".into(),
                enabled,
                activate: Box::new(|this: &mut Self| {
                    if let Some(text) = click_text(&this.recent, 0) {
                        (this.actions)(TrayAction::Copy(text.to_string()));
                    }
                }),
                ..Default::default()
            }
            .into(),
        }
    }
}

/// Read the recent dictations: `[history]` as the config file says now
/// (the daemon re-reads it live), the history store, and
/// `last-dictation.json`. Blocking file reads.
fn load_recent(recorder: &mut Recorder, limit: usize) -> Vec<Recent> {
    let recording = recorder.refresh().recording();
    let entries = if recording {
        HistoryStore::at(recorder.store_dir())
            .recent(limit)
            .unwrap_or_else(|e| {
                tracing::warn!("Tray could not read history: {e}");
                Vec::new()
            })
    } else {
        Vec::new()
    };
    recent_dictations(recording, entries, crate::last_dictation::read(), limit)
}

async fn load_recent_async(mut recorder: Recorder, limit: usize) -> (Recorder, Vec<Recent>) {
    let fallback = recorder.clone();
    tokio::task::spawn_blocking(move || {
        let recent = load_recent(&mut recorder, limit);
        (recorder, recent)
    })
    .await
    .unwrap_or((fallback, Vec::new()))
}

/// Explicit shutdown is essential: dropping a ksni Handle alone does not close it.
struct ServiceGuard(ksni::Handle<VoxtypeTray>);
impl Drop for ServiceGuard {
    fn drop(&mut self) {
        drop(self.0.shutdown());
    }
}

fn spawn_inner(
    mut rx: watch::Receiver<TrayState>,
    actions: Actions,
    mut recorder: Recorder,
    shown: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Copy last dictation needs the newest one even with the list hidden.
        let limit = shown.max(1);
        let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel();
        let (loaded, recent) = load_recent_async(recorder, limit).await;
        recorder = loaded;
        let tray = VoxtypeTray {
            state: *rx.borrow_and_update(),
            recent,
            shown,
            actions,
            refresh: refresh_tx,
        };
        let start = tray.assume_sni_available(true).spawn();
        // Bounded, cancellable startup. D-Bus must never hold dictation startup hostage.
        let handle = tokio::select! {
            _ = rx.wait_for(|_| false) => return,
            result = tokio::time::timeout(Duration::from_secs(5), start) => match result {
                Ok(Ok(h)) => ServiceGuard(h),
                Ok(Err(error)) => { tracing::warn!(?error, "Tray unavailable; dictation continues"); return; }
                Err(_) => { tracing::warn!("Tray startup timed out; dictation continues"); return; }
            }
        };
        // Reflect a state that arrived during registration.
        let state = *rx.borrow_and_update();
        handle.0.update(|tray| tray.state = state).await;
        // The state channel alone decides the tray's lifetime.
        loop {
            let reload = tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let state = *rx.borrow_and_update();
                    if handle.0.update(|tray| tray.state = state).await.is_none() {
                        break;
                    }
                    // A dictation that just ended changes the list.
                    matches!(state, TrayState::Idle | TrayState::Error)
                }
                Some(()) = refresh_rx.recv() => true,
            };
            if reload {
                let (loaded, recent) = load_recent_async(recorder, limit).await;
                recorder = loaded;
                let updated = handle
                    .0
                    .update(|tray| {
                        if tray.recent != recent {
                            tray.recent = recent;
                        }
                    })
                    .await;
                if updated.is_none() {
                    break;
                }
            }
        }
        handle.0.shutdown().await;
    })
}

fn dispatch_action(action: TrayAction) {
    tokio::spawn(async move {
        if let TrayAction::Copy(text) = &action {
            if let Err(e) = copy(text).await {
                tracing::warn!("Tray could not copy the dictation: {e}");
            }
            return;
        }
        let result = async {
            let bin = std::env::current_exe()?;
            Command::new(bin)
                .args(action.cli_args().unwrap_or_default())
                .stdin(std::process::Stdio::null())
                .status()
                .await
        }
        .await;
        match result {
            Ok(status) if status.success() => {}
            error => tracing::warn!(?action, ?error, "Tray action failed"),
        }
    });
}

/// Put `text` on the clipboard with the first clipboard driver that works,
/// as `voxtype history copy` does.
async fn copy(text: &str) -> Result<&'static str, String> {
    let clipboard = crate::config::OutputConfig {
        mode: crate::config::OutputMode::Clipboard,
        append_text: None,
        ..Default::default()
    };
    let mut errors = Vec::new();
    for driver in crate::output::create_output_chain(&clipboard) {
        if driver.is_available().await {
            match driver.output(text).await {
                Ok(()) => return Ok(driver.name()),
                Err(e) => errors.push(e.to_string()),
            }
        }
    }
    Err(format!(
        "no clipboard driver succeeded ({})",
        errors.join("; ")
    ))
}

/// Daemon-owned guard. Early errors/cancellation also request service shutdown.
pub struct TrayRuntime {
    tx: Option<watch::Sender<TrayState>>,
    task: tokio::task::JoinHandle<()>,
}

impl TrayRuntime {
    /// The daemon's tray, or `None` while `[tray] enabled` is off.
    /// `config_path` is the config the daemon runs on, for re-reading
    /// `[history]`.
    pub fn start_for_daemon(
        config: &crate::config::Config,
        config_path: Option<std::path::PathBuf>,
    ) -> Option<Self> {
        if !config.tray.enabled {
            return None;
        }
        let (tx, rx) = watch::channel(TrayState::Idle);
        let recorder = Recorder::new(config, config_path);
        let task = spawn_inner(
            rx,
            Arc::new(dispatch_action),
            recorder,
            config.tray.recent as usize,
        );
        Some(Self { tx: Some(tx), task })
    }

    pub fn update(&self, state: TrayState) {
        if let Some(tx) = &self.tx {
            tx.send_if_modified(|old| {
                if *old == state {
                    false
                } else {
                    *old = state;
                    true
                }
            });
        }
    }

    pub async fn shutdown(mut self) {
        self.tx.take();
        if tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}

impl Drop for TrayRuntime {
    fn drop(&mut self) {
        self.tx.take();
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn entry(hour: u32, id: &str, text: &str) -> HistoryEntry {
        let start = Local.with_ymd_and_hms(2026, 11, 20, hour, 0, 0).unwrap();
        let mut e = HistoryEntry::new(start);
        e.id = id.to_string();
        e.text = text.to_string();
        e
    }

    fn kept(text: &str) -> LastDictation {
        LastDictation {
            text: text.to_string(),
            time: "2026-11-20T12:00:00+01:00".into(),
            delivered: true,
            method: Some("paste".into()),
            error: None,
        }
    }

    fn texts(recent: &[Recent]) -> Vec<&str> {
        recent.iter().map(|r| r.text.as_str()).collect()
    }

    fn recent_labels(items: &[Item]) -> Option<&Vec<String>> {
        items.iter().find_map(|i| match i {
            Item::Recent(labels) => Some(labels),
            _ => None,
        })
    }

    /// Three entries handed over oldest first, as a store read might.
    fn three() -> Vec<HistoryEntry> {
        vec![
            entry(9, "a", "The quick brown fox."),
            entry(10, "b", "Jumps over the lazy dog."),
            entry(11, "c", "Pack my box with five dozen jugs."),
        ]
    }

    #[test]
    fn recent_lists_history_newest_first() {
        let recent = recent_dictations(true, three(), Some(kept("A kept line.")), 10);
        assert_eq!(
            texts(&recent),
            [
                "Pack my box with five dozen jugs.",
                "Jumps over the lazy dog.",
                "The quick brown fox."
            ]
        );
        assert_eq!(recent[0].id.as_deref(), Some("c"));
        let items = menu_items(TrayState::Idle, &recent, 10);
        assert_eq!(
            recent_labels(&items).unwrap(),
            &[
                "Pack my box with five dozen jugs.",
                "Jumps over the lazy dog.",
                "The quick brown fox."
            ]
        );
    }

    #[test]
    fn recent_keeps_the_newest_up_to_the_limit() {
        let recent = recent_dictations(true, three(), None, 2);
        assert_eq!(
            texts(&recent),
            [
                "Pack my box with five dozen jugs.",
                "Jumps over the lazy dog."
            ]
        );
        let recent = recent_dictations(true, three(), None, 10);
        let items = menu_items(TrayState::Idle, &recent, 1);
        assert_eq!(
            recent_labels(&items).unwrap(),
            &["Pack my box with five dozen jugs."]
        );
    }

    #[test]
    fn without_history_the_kept_dictation_is_the_only_entry() {
        let recent = recent_dictations(false, three(), Some(kept("A kept line.")), 10);
        assert_eq!(texts(&recent), ["A kept line."]);
        assert_eq!(recent[0].id, None);
        // Recording but empty history: the kept one.
        let recent = recent_dictations(true, Vec::new(), Some(kept("A kept line.")), 10);
        assert_eq!(texts(&recent), ["A kept line."]);
        assert!(recent_dictations(false, Vec::new(), None, 10).is_empty());
        assert!(recent_dictations(false, Vec::new(), Some(kept("  ")), 10).is_empty());
    }

    #[test]
    fn entries_without_text_are_skipped() {
        let mut entries = three();
        entries.push(entry(12, "d", ""));
        let recent = recent_dictations(true, entries, None, 10);
        assert_eq!(recent[0].text, "Pack my box with five dozen jugs.");
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn a_click_copies_the_entry_it_shows() {
        let recent = recent_dictations(true, three(), None, 10);
        let items = menu_items(TrayState::Idle, &recent, 10);
        let labels = recent_labels(&items).unwrap();
        for (k, shown) in labels.iter().enumerate() {
            let copied = click_text(&recent, k).unwrap();
            assert_eq!(copied, recent[k].text);
            assert_eq!(&label(copied), shown);
        }
        assert_eq!(click_text(&recent, 1), Some("Jumps over the lazy dog."));
        assert_eq!(click_text(&recent, 3), None);
    }

    #[test]
    fn labels_are_one_line_of_forty_characters() {
        assert_eq!(label("The quick brown fox."), "The quick brown fox.");
        assert_eq!(label("The quick\nbrown\tfox."), "The quick brown fox.");
        let long = "The quick brown fox jumps over the lazy dog again and again.";
        let l = label(long);
        assert_eq!(l, "The quick brown fox jumps over the lazy…");
        assert_eq!(l.chars().count(), 40);
        // Exactly forty characters is not cut.
        let forty = "a".repeat(40);
        assert_eq!(label(&forty), forty);
        // Counted in characters, not bytes.
        let wide = "æ".repeat(41);
        assert_eq!(label(&wide), format!("{}…", "æ".repeat(40)));
    }

    #[test]
    fn recent_zero_hides_the_submenu_but_keeps_copy_last() {
        let recent = recent_dictations(true, three(), None, 1);
        let items = menu_items(TrayState::Idle, &recent, 0);
        assert!(recent_labels(&items).is_none());
        assert!(items.contains(&Item::CopyLast { enabled: true }));
        let items = menu_items(TrayState::Idle, &[], 10);
        assert_eq!(recent_labels(&items).unwrap().len(), 0);
        assert!(items.contains(&Item::CopyLast { enabled: false }));
    }

    #[test]
    fn toggle_and_cancel_follow_the_state() {
        let has = |state, item: Item| menu_items(state, &[], 10).contains(&item);
        assert!(has(
            TrayState::Idle,
            Item::Toggle {
                stop: false,
                enabled: true
            }
        ));
        assert!(has(TrayState::Idle, Item::Cancel { enabled: false }));
        assert!(has(
            TrayState::Recording,
            Item::Toggle {
                stop: true,
                enabled: true
            }
        ));
        assert!(has(TrayState::Recording, Item::Cancel { enabled: true }));
        assert!(has(
            TrayState::Transcribing,
            Item::Toggle {
                stop: false,
                enabled: false
            }
        ));
        assert!(has(TrayState::Transcribing, Item::Cancel { enabled: true }));
        assert!(has(
            TrayState::Error,
            Item::Toggle {
                stop: false,
                enabled: true
            }
        ));
        assert_eq!(
            menu_items(TrayState::Recording, &[], 10)[0],
            Item::Status("Voxtype: Recording...".into())
        );
    }

    #[test]
    fn daemon_state_names_map_to_tray_states() {
        assert_eq!(TrayState::from_state_name("idle"), TrayState::Idle);
        assert_eq!(
            TrayState::from_state_name("streaming"),
            TrayState::Recording
        );
        assert_eq!(
            TrayState::from_state_name("Transcribing\n"),
            TrayState::Transcribing
        );
        assert_eq!(TrayState::from_state_name("error"), TrayState::Error);
        assert_eq!(TrayState::from_state_name("stopped"), TrayState::Stopped);
        assert_eq!(TrayState::from_state_name("unknown"), TrayState::Idle);
        let names = [
            TrayState::Idle,
            TrayState::Recording,
            TrayState::Transcribing,
            TrayState::Error,
        ]
        .map(TrayState::icon_name);
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn actions_run_fixed_commands() {
        assert_eq!(
            TrayAction::ToggleRecording.cli_args(),
            Some(&["record", "toggle"][..])
        );
        assert_eq!(
            TrayAction::Cancel.cli_args(),
            Some(&["record", "cancel"][..])
        );
        assert_eq!(TrayAction::Copy("x".into()).cli_args(), None);
        assert_eq!(escape("snake_case"), "snake__case");
    }
}
