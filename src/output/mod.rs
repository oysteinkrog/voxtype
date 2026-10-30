//! Text output module
//!
//! Provides text output via keyboard simulation or clipboard.
//!
//! Fallback chain for `mode = "type"`:
//!
//! Linux:
//! 1. wtype - Wayland-native via virtual-keyboard protocol, best Unicode/CJK support, no daemon needed
//! 2. eitype - Wayland via libei/EI protocol, works on GNOME/KDE (no virtual-keyboard support)
//! 3. dotool - Works on X11/Wayland/TTY, supports keyboard layouts, no daemon needed
//! 4. ydotool - Works on X11/Wayland/TTY, requires daemon
//! 5. clipboard (wl-copy) - Wayland clipboard fallback
//! 6. xclip - X11 clipboard fallback
//!
//! macOS:
//! 1. cgevent - Native CGEvent API for keyboard simulation (best performance)
//! 2. osascript - AppleScript fallback
//! 3. pbcopy - Native macOS clipboard
//!
//! Paste mode (clipboard + Ctrl+V) helps with system with non US keyboard layouts.

#[cfg(target_os = "macos")]
pub mod cgevent;
pub mod clipboard;
pub mod dotool;
pub mod eitype;
// modifier_guard is evdev-based; macOS has its own osascript modifier handling.
#[cfg(target_os = "linux")]
pub mod modifier_guard;
#[cfg(target_os = "macos")]
pub mod osascript;
pub mod paste;
#[cfg(target_os = "macos")]
pub mod pbcopy;
pub mod post_process;
pub mod session;
pub mod streaming;
pub mod wtype;
pub mod xclip;
pub mod ydotool;

pub use streaming::StreamingSession;

use crate::config::{OutputConfig, OutputDriver};
use crate::error::OutputError;
use std::borrow::Cow;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// Find the ydotool daemon socket by checking known locations.
///
/// Fedora places the socket at `/tmp/.ydotool_socket`, while the ydotool CLI
/// defaults to `$XDG_RUNTIME_DIR/.ydotool_socket`. Without setting `YDOTOOL_SOCKET`
/// explicitly, ydotool commands fail on distros that use a non-default path.
///
/// Search order:
/// 1. `$YDOTOOL_SOCKET` env var (user override)
/// 2. `$XDG_RUNTIME_DIR/.ydotool_socket` (ydotool CLI default)
/// 3. `/tmp/.ydotool_socket` (Fedora / systemd-wide service)
/// 4. `/run/user/$UID/.ydotool_socket` (fallback if XDG_RUNTIME_DIR unset)
pub fn find_ydotool_socket() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = {
        let mut paths = Vec::new();

        // 1. Explicit env override
        if let Ok(val) = std::env::var("YDOTOOL_SOCKET") {
            paths.push(PathBuf::from(val));
        }

        // 2. XDG_RUNTIME_DIR (ydotool CLI default)
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            paths.push(PathBuf::from(xdg).join(".ydotool_socket"));
        }

        // 3. /tmp (Fedora systemd-wide ydotoold)
        paths.push(PathBuf::from("/tmp/.ydotool_socket"));

        // 4. /run/user/$UID fallback
        let uid = unsafe { libc::getuid() };
        paths.push(PathBuf::from(format!("/run/user/{}/.ydotool_socket", uid)));

        paths
    };

    for path in &candidates {
        if let Ok(meta) = fs::metadata(path) {
            if meta.file_type().is_socket() {
                tracing::debug!("Found ydotool socket at {}", path.display());
                return Some(path.clone());
            }
        }
    }

    tracing::debug!(
        "No ydotool socket found in: {}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    None
}

/// Normalize Unicode curly quotes to ASCII equivalents.
///
/// Whisper sometimes outputs curly/smart quotes which can cause issues with
/// keyboard simulation tools (wtype, dotool, ydotool). This function converts
/// them to standard ASCII quotes to prevent unexpected line breaks or other
/// typing artifacts.
pub(crate) fn normalize_quotes(text: &str) -> Cow<'_, str> {
    // Quick check to avoid allocation if no normalization needed
    let needs_normalization = text.chars().any(|c| {
        matches!(
            c,
            '\u{2018}'  // LEFT SINGLE QUOTATION MARK
            | '\u{2019}'  // RIGHT SINGLE QUOTATION MARK (curly apostrophe)
            | '\u{201B}'  // SINGLE HIGH-REVERSED-9 QUOTATION MARK
            | '\u{2032}'  // PRIME
            | '\u{201C}'  // LEFT DOUBLE QUOTATION MARK
            | '\u{201D}'  // RIGHT DOUBLE QUOTATION MARK
            | '\u{201F}'  // DOUBLE HIGH-REVERSED-9 QUOTATION MARK
            | '\u{2033}' // DOUBLE PRIME
        )
    });

    if !needs_normalization {
        return Cow::Borrowed(text);
    }

    Cow::Owned(
        text.chars()
            .map(|c| match c {
                // Single quotes/apostrophes -> ASCII apostrophe
                '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
                // Double quotes -> ASCII double quote
                '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
                other => other,
            })
            .collect(),
    )
}

/// Path to the voxtype symlink
const VOXTYPE_BIN: &str = "/usr/lib/voxtype/voxtype";

/// Check if the active binary is a Parakeet build
pub fn is_parakeet_binary_active() -> bool {
    if let Ok(link_target) = fs::read_link(VOXTYPE_BIN) {
        if let Some(target_name) = link_target.file_name() {
            if let Some(name) = target_name.to_str() {
                return name.contains("onnx") || name.contains("parakeet");
            }
        }
    }
    // If we can't read the symlink, check if parakeet feature is enabled
    #[cfg(feature = "parakeet")]
    {
        return true;
    }
    #[cfg(not(feature = "parakeet"))]
    {
        false
    }
}

/// Get the engine icon for notifications based on configured engine
pub fn engine_icon(engine: crate::config::TranscriptionEngine) -> &'static str {
    match engine {
        crate::config::TranscriptionEngine::Parakeet => "\u{1F99C}", // 🦜
        crate::config::TranscriptionEngine::Whisper => "\u{1F5E3}\u{FE0F}", // 🗣️
        crate::config::TranscriptionEngine::Moonshine => "\u{1F319}", // 🌙
        crate::config::TranscriptionEngine::SenseVoice => "\u{1F442}", // 👂
        crate::config::TranscriptionEngine::Paraformer => "\u{1F4AC}", // 💬
        crate::config::TranscriptionEngine::Dolphin => "\u{1F42C}",  // 🐬
        crate::config::TranscriptionEngine::Omnilingual => "\u{1F30D}", // 🌍
        crate::config::TranscriptionEngine::Cohere => "\u{1F4DD}",   // 📝
        crate::config::TranscriptionEngine::Soniox => "\u{2601}\u{FE0F}", // ☁️
        crate::config::TranscriptionEngine::OpenVino => "\u{1F9E0}", // 🧠
    }
}

/// Validate notification urgency, falling back to "normal" for unknown values.
///
/// notify-send only accepts "low", "normal", or "critical".
pub fn sanitize_urgency(urgency: &str) -> &str {
    match urgency {
        "low" | "normal" | "critical" => urgency,
        _ => "normal",
    }
}

/// Send a transcription notification with optional engine icon
pub async fn send_transcription_notification(
    text: &str,
    show_engine_icon: bool,
    engine: crate::config::TranscriptionEngine,
    urgency: &str,
) {
    // Truncate preview for notification (use chars() to handle multi-byte UTF-8)
    let preview = if text.chars().count() > 80 {
        format!("{}...", text.chars().take(80).collect::<String>())
    } else {
        text.to_string()
    };

    let title = if show_engine_icon {
        format!("{} Transcribed", engine_icon(engine))
    } else {
        "Transcribed".to_string()
    };

    // Through the notification module rather than straight to notify-send:
    // that module owns the --replace-id bookkeeping that keeps every Voxtype
    // notification in a single slot, and a notification posted from here
    // stacks beside the daemon's instead of replacing it ([#532]).
    crate::notification::send_status(
        &title,
        &preview,
        urgency,
        crate::notification::Lifetime::Millis(3000),
    )
    .await;
}

/// Trait for text output implementations
#[async_trait::async_trait]
#[allow(clippy::double_must_use)] // async_trait adds its own must_use
pub trait TextOutput: Send + Sync {
    /// Output text (type it or copy to clipboard)
    async fn output(&self, text: &str) -> Result<(), OutputError>;

    /// Check if this output method is available
    async fn is_available(&self) -> bool;

    /// Human-readable name for logging
    fn name(&self) -> &'static str;
}

/// Default driver order for type mode
#[cfg(not(target_os = "macos"))]
const DEFAULT_DRIVER_ORDER: &[OutputDriver] = &[
    OutputDriver::Wtype,
    OutputDriver::Eitype,
    OutputDriver::Dotool,
    OutputDriver::Ydotool,
    OutputDriver::Clipboard,
    OutputDriver::Xclip,
];

/// Create a TextOutput implementation for a specific driver
#[cfg(not(target_os = "macos"))]
fn create_driver_output(
    driver: OutputDriver,
    config: &OutputConfig,
    pre_type_delay_ms: u32,
) -> Box<dyn TextOutput> {
    match driver {
        OutputDriver::Wtype => Box::new(wtype::WtypeOutput::new(
            config.auto_submit,
            config.append_text.clone(),
            config.type_delay_ms,
            pre_type_delay_ms,
            config.shift_enter_newlines,
            config.wtype_shift_prefix,
        )),
        OutputDriver::Eitype => Box::new(eitype::EitypeOutput::new(
            config.auto_submit,
            config.append_text.clone(),
            config.type_delay_ms,
            pre_type_delay_ms,
            config.shift_enter_newlines,
            config.eitype_xkb_layout.clone(),
            config.eitype_xkb_variant.clone(),
        )),
        OutputDriver::Dotool => Box::new(dotool::DotoolOutput::new(
            config.type_delay_ms,
            pre_type_delay_ms,
            config.auto_submit,
            config.append_text.clone(),
            config.dotool_xkb_layout.clone(),
            config.dotool_xkb_variant.clone(),
        )),
        OutputDriver::Ydotool => Box::new(ydotool::YdotoolOutput::new(
            config.type_delay_ms,
            pre_type_delay_ms,
            config.auto_submit,
            config.append_text.clone(),
        )),
        OutputDriver::Clipboard => {
            Box::new(clipboard::ClipboardOutput::new(config.append_text.clone()))
        }
        OutputDriver::Xclip => Box::new(xclip::XclipOutput::new(config.append_text.clone())),
    }
}

/// Factory function that returns a fallback chain of output methods
pub fn create_output_chain(config: &OutputConfig) -> Vec<Box<dyn TextOutput>> {
    create_output_chain_with_override(config, None)
}

/// Factory function that returns a fallback chain of output methods with an optional driver override
pub fn create_output_chain_with_override(
    config: &OutputConfig,
    driver_override: Option<&[OutputDriver]>,
) -> Vec<Box<dyn TextOutput>> {
    let mut chain: Vec<Box<dyn TextOutput>> = Vec::new();
    #[cfg(target_os = "macos")]
    let _ = driver_override;

    // Get effective pre_type_delay_ms (handles deprecated wtype_delay_ms)
    let pre_type_delay_ms = config.effective_pre_type_delay_ms();

    match config.mode {
        crate::config::OutputMode::Type => {
            #[cfg(target_os = "macos")]
            {
                // macOS: Primary - CGEvent (native API, best performance)
                // driver_order not yet supported on macOS
                let show_notification = config.notification.on_transcription;
                chain.push(Box::new(cgevent::CGEventOutput::new(
                    config.type_delay_ms,
                    pre_type_delay_ms,
                    show_notification,
                    config.auto_submit,
                )));

                // Fallback 1: osascript (AppleScript, works without CGEvent permissions)
                chain.push(Box::new(osascript::OsascriptOutput::new(
                    false, // notification already handled by primary
                    config.auto_submit,
                    pre_type_delay_ms,
                )));

                // Fallback 2: pbcopy for clipboard
                if config.fallback_to_clipboard {
                    chain.push(Box::new(pbcopy::PbcopyOutput::new(false)));
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                // Determine driver order: CLI override > config > default
                let driver_order: &[OutputDriver] = driver_override
                    .or(config.driver_order.as_deref())
                    .unwrap_or(DEFAULT_DRIVER_ORDER);

                if let Some(custom_order) = driver_override.or(config.driver_order.as_deref()) {
                    tracing::info!(
                        "Using custom driver order: {}",
                        custom_order
                            .iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    );
                }

                for driver in driver_order.iter() {
                    chain.push(create_driver_output(*driver, config, pre_type_delay_ms));
                }

                // If fallback_to_clipboard is true but clipboard wasn't in the custom order, add it
                if config.fallback_to_clipboard
                    && config.driver_order.is_some()
                    && !driver_order.contains(&OutputDriver::Clipboard)
                {
                    chain.push(Box::new(clipboard::ClipboardOutput::new(
                        config.append_text.clone(),
                    )));
                }
            }
        }
        crate::config::OutputMode::Clipboard => {
            #[cfg(target_os = "macos")]
            chain.push(Box::new(pbcopy::PbcopyOutput::new(
                config.notification.on_transcription,
            )));

            #[cfg(not(target_os = "macos"))]
            {
                // Clipboard with X11 fallback: wl-copy first, then xclip
                chain.push(Box::new(clipboard::ClipboardOutput::new(
                    config.append_text.clone(),
                )));
                chain.push(Box::new(xclip::XclipOutput::new(
                    config.append_text.clone(),
                )));
            }
        }
        crate::config::OutputMode::Paste => {
            // Only paste mode (no fallback as requested)
            chain.push(Box::new(
                paste::PasteOutput::new(
                    config.auto_submit,
                    config.append_text.clone(),
                    config.paste_keys.clone(),
                    config.driver_order.clone(),
                    config.type_delay_ms,
                    pre_type_delay_ms,
                    config.restore_clipboard,
                    config.restore_clipboard_delay_ms,
                )
                .with_paste_primary(config.paste_primary),
            ));
        }
        crate::config::OutputMode::File => {
            // The daemon writes the file itself before reaching the output
            // chain, so with a file_path this chain only backs the startup
            // log. Warn only when there is genuinely nowhere to write.
            if file_mode_lacks_path(config) {
                tracing::warn!(
                    "Output mode is 'file' but no file_path configured. Falling back to clipboard."
                );
            }
            chain.push(Box::new(clipboard::ClipboardOutput::new(
                config.append_text.clone(),
            )));
        }
    }

    chain
}

/// True when mode = "file" has no file_path to write to, so the output
/// chain's clipboard fallback is what actually delivers the text.
fn file_mode_lacks_path(config: &OutputConfig) -> bool {
    config.mode == crate::config::OutputMode::File && config.file_path.is_none()
}

/// Run a shell command (for pre/post hooks)
pub async fn run_hook(command: &str, hook_name: &str) -> Result<(), String> {
    run_hook_with(command, hook_name, None).await
}

/// How a dictation ended, for the output hooks as `VOXTYPE_OUTCOME`, so a
/// hook that presses Enter can leave it out when no text arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookOutcome {
    /// The text was delivered (in the pre-output hook: is about to be).
    Done,
    /// The dictation was cancelled.
    Cancelled,
    /// Nothing was heard or transcribed.
    Empty,
    /// Recording, transcription or output failed.
    Failed,
    /// The text is on the clipboard only, not typed or pasted.
    NotPasted,
}

impl HookOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            HookOutcome::Done => "done",
            HookOutcome::Cancelled => "cancelled",
            HookOutcome::Empty => "empty",
            HookOutcome::Failed => "failed",
            HookOutcome::NotPasted => "not-pasted",
        }
    }
}

/// [`run_hook`] for `pre_output_command` and `post_output_command`, with
/// `VOXTYPE_OUTCOME` set to `outcome`.
pub async fn run_output_hook(
    command: &str,
    hook_name: &str,
    outcome: HookOutcome,
) -> Result<(), String> {
    run_hook_with(command, hook_name, Some(outcome)).await
}

async fn run_hook_with(
    command: &str,
    hook_name: &str,
    outcome: Option<HookOutcome>,
) -> Result<(), String> {
    tracing::debug!("Running {} hook: {}", hook_name, command);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(outcome) = outcome {
        cmd.env("VOXTYPE_OUTCOME", outcome.as_str());
    }
    let output = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("{} hook failed to execute: {}", hook_name, e))?;

    if output.status.success() {
        tracing::info!("{} hook completed successfully", hook_name);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{} hook failed: {}", hook_name, stderr))
    }
}

/// Output configuration for the fallback chain
pub struct OutputOptions<'a> {
    pub pre_output_command: Option<&'a str>,
    pub post_output_command: Option<&'a str>,
    /// Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before
    /// invoking keystroke-synthesizing output methods.
    pub wait_for_modifier_release: bool,
    /// Maximum time to wait for modifier release before skipping keystroke
    /// methods and falling through to clipboard-only methods.
    pub modifier_release_timeout: std::time::Duration,
    /// Turn curly quotes into ASCII first. Off for text that must arrive
    /// exactly as shown, such as keyboard-edited text.
    pub normalize: bool,
}

/// Output methods that synthesize keystrokes the compositor can interpret as
/// keybindings when modifiers are held. Used to filter the chain when the
/// modifier-release wait times out.
fn is_keystroke_method(name: &str) -> bool {
    matches!(name, "wtype" | "eitype" | "dotool" | "ydotool") || name.starts_with("paste")
}

/// True when any method in the chain synthesizes keystrokes, so the
/// modifier-release guard has something to protect. Clipboard-only chains
/// (clipboard mode, or a `driver_order` of clipboard drivers) never press a
/// key, and building the guard costs hundreds of milliseconds on hosts with
/// many input devices (#543), so they skip it.
fn chain_synthesizes_keystrokes<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    names.into_iter().any(is_keystroke_method)
}

/// Try each output method in the chain until one succeeds
/// Pre/post output commands are run before and after typing (for compositor integration).
pub async fn output_with_fallback(
    chain: &[Box<dyn TextOutput>],
    text: &str,
    options: OutputOptions<'_>,
) -> Result<(), OutputError> {
    output_via(chain, text, options).await.map(|_| ())
}

/// Like [`output_with_fallback`], and returns the name of the method that
/// delivered the text.
pub async fn output_via(
    chain: &[Box<dyn TextOutput>],
    text: &str,
    options: OutputOptions<'_>,
) -> Result<&'static str, OutputError> {
    output_via_confirmed(chain, text, options, || Box::pin(async { true })).await
}

/// Like [`output_via`], and asks `confirm` after the pre-output hook, right
/// before the text is typed, and again right before the post-output hook.
/// After a keyboard edit nothing may be typed, and no hook may press Enter,
/// once the target window lost focus (plan R28). A no before typing gives
/// [`OutputError::TargetChanged`] and runs no post-output hook; a no before
/// the hook skips it. The second ask comes only with a post-output hook.
pub async fn output_via_confirmed(
    chain: &[Box<dyn TextOutput>],
    text: &str,
    options: OutputOptions<'_>,
    confirm: impl Fn() -> futures_util::future::BoxFuture<'static, bool>,
) -> Result<&'static str, OutputError> {
    // Normalize curly quotes to ASCII to prevent line break issues with keyboard tools
    let normalized_text = if options.normalize {
        normalize_quotes(text)
    } else {
        Cow::Borrowed(text)
    };

    // If the modifier guard is enabled, snapshot kernel-level key state and
    // wait for any held modifiers to be released. This prevents typed letters
    // from combining with Super/Ctrl/Alt/Shift and firing compositor or
    // application keybindings. Disabled and timed-out cases both leave the
    // chain runnable; only the latter skips keystroke-synthesizing methods.
    let mut skip_keystroke_methods = false;
    // The evdev-based modifier_guard is Linux-only. macOS uses its own
    // wait_for_modifiers_release in osascript.rs and gets called separately
    // from the osascript output path.
    #[cfg(target_os = "linux")]
    if options.wait_for_modifier_release
        && chain_synthesizes_keystrokes(chain.iter().map(|o| o.name()))
    {
        let mut guard = modifier_guard::ModifierGuard::new();
        if guard
            .wait_for_release(options.modifier_release_timeout)
            .await
            .is_err()
        {
            tracing::warn!(
                timeout_ms = options.modifier_release_timeout.as_millis() as u64,
                "Modifier keys still held after timeout; skipping \
                 keystroke-synthesizing methods and using clipboard fallback \
                 to avoid triggering keybindings"
            );
            // Surface the fallback to the user so they know where the
            // transcription went. Silent clipboard fallback leaves users
            // staring at an empty cursor wondering why nothing was typed.
            crate::notification::send(
                "Voxtype",
                "Modifier key held too long, transcription copied to clipboard.",
            )
            .await;
            skip_keystroke_methods = true;
        }
    }

    // Run pre-output hook if configured (e.g., switch to modifier-suppressing submap)
    if let Some(cmd) = options.pre_output_command {
        if let Err(e) = run_output_hook(cmd, "pre_output", HookOutcome::Done).await {
            tracing::warn!("{}", e);
            // Continue anyway - best effort
        }
    }

    // The hook can take a while or move focus itself.
    if !confirm().await {
        tracing::warn!("Target window changed before the text was typed; nothing typed");
        return Err(OutputError::TargetChanged);
    }

    // Try each output method
    let mut result = Err(OutputError::AllMethodsFailed);
    for output in chain {
        if skip_keystroke_methods && is_keystroke_method(output.name()) {
            tracing::debug!(
                "{} skipped (modifier still held), trying next",
                output.name()
            );
            continue;
        }

        if !output.is_available().await {
            tracing::debug!("{} not available, trying next", output.name());
            continue;
        }

        match output.output(&normalized_text).await {
            Ok(()) => {
                tracing::debug!("Text output via {}", output.name());
                result = Ok(output.name());
                break;
            }
            Err(e) => {
                tracing::warn!("{} failed: {}, trying next", output.name(), e);
            }
        }
    }

    // The text went to the clipboard although the chain could type it: the
    // keystroke methods were skipped or failed.
    let outcome = match result {
        Ok(name)
            if name.starts_with("clipboard")
                && chain_synthesizes_keystrokes(chain.iter().map(|o| o.name())) =>
        {
            HookOutcome::NotPasted
        }
        Ok(_) => HookOutcome::Done,
        Err(_) => HookOutcome::Failed,
    };

    // Run post-output hook if configured (e.g., reset submap)
    // Always run this, even on failure, to ensure cleanup, unless `confirm`
    // says the target is gone.
    if let Some(cmd) = options.post_output_command {
        if !confirm().await {
            tracing::warn!("Target window changed before the post-output hook; skipping it");
        } else if let Err(e) = run_output_hook(cmd, "post_output", outcome).await {
            tracing::warn!("{}", e);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An output method that succeeds or fails, under a given name.
    struct Named(&'static str, bool);

    #[async_trait::async_trait]
    impl TextOutput for Named {
        async fn output(&self, _text: &str) -> Result<(), OutputError> {
            if self.1 {
                Ok(())
            } else {
                Err(OutputError::AllMethodsFailed)
            }
        }
        async fn is_available(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            self.0
        }
    }

    /// The post-output hook's VOXTYPE_OUTCOME for `chain`.
    async fn post_outcome(chain: Vec<Box<dyn TextOutput>>) -> String {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("outcome");
        let hook = format!("echo $VOXTYPE_OUTCOME > '{}'", log.display());
        let _ = output_via(
            &chain,
            "text",
            OutputOptions {
                pre_output_command: None,
                post_output_command: Some(&hook),
                wait_for_modifier_release: false,
                modifier_release_timeout: std::time::Duration::ZERO,
                normalize: false,
            },
        )
        .await;
        std::fs::read_to_string(&log).unwrap().trim().to_string()
    }

    #[tokio::test]
    async fn the_post_output_hook_gets_the_outcome() {
        assert_eq!(
            post_outcome(vec![Box::new(Named("wtype", true))]).await,
            "done"
        );
        // Clipboard mode: copying is the delivery.
        assert_eq!(
            post_outcome(vec![Box::new(Named("clipboard (wl-copy)", true))]).await,
            "done"
        );
        // Typing failed and the clipboard took the text: no Enter.
        assert_eq!(
            post_outcome(vec![
                Box::new(Named("wtype", false)),
                Box::new(Named("clipboard (wl-copy)", true)),
            ])
            .await,
            "not-pasted"
        );
        assert_eq!(
            post_outcome(vec![Box::new(Named("wtype", false))]).await,
            "failed"
        );
    }

    #[test]
    fn file_mode_warns_only_without_a_path() {
        let mut config = OutputConfig {
            mode: crate::config::OutputMode::File,
            ..Default::default()
        };
        assert!(file_mode_lacks_path(&config));

        config.file_path = Some(PathBuf::from("/tmp/notes.txt"));
        assert!(!file_mode_lacks_path(&config));

        config.file_path = None;
        config.mode = crate::config::OutputMode::Clipboard;
        assert!(!file_mode_lacks_path(&config));
    }

    #[test]
    fn test_normalize_quotes_no_change() {
        let text = "Hello, world! It's a test.";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, text);
    }

    #[test]
    fn test_normalize_quotes_curly_apostrophe() {
        let text = "It\u{2019}s a test";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Owned(_)));
        assert_eq!(result, "It's a test");
    }

    #[test]
    fn test_normalize_quotes_all_single() {
        let text = "\u{2018}hello\u{2019} \u{201B}world\u{2032}";
        let result = normalize_quotes(text);
        assert_eq!(result, "'hello' 'world'");
    }

    #[test]
    fn test_normalize_quotes_all_double() {
        let text = "\u{201C}hello\u{201D} \u{201F}world\u{2033}";
        let result = normalize_quotes(text);
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_normalize_quotes_mixed() {
        let text = "\u{201C}Don\u{2019}t worry,\u{201D} she said.";
        let result = normalize_quotes(text);
        assert_eq!(result, "\"Don't worry,\" she said.");
    }

    #[test]
    fn test_normalize_quotes_empty() {
        let text = "";
        let result = normalize_quotes(text);
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "");
    }

    #[test]
    fn test_normalize_quotes_unicode_preserved() {
        let text = "Café \u{2019} emoji 😀";
        let result = normalize_quotes(text);
        assert_eq!(result, "Café ' emoji 😀");
    }

    #[test]
    fn test_is_keystroke_method_classification() {
        assert!(is_keystroke_method("wtype"));
        assert!(is_keystroke_method("eitype"));
        assert!(is_keystroke_method("dotool"));
        assert!(is_keystroke_method("ydotool"));
        assert!(is_keystroke_method("paste (clipboard + keystroke)"));
        assert!(!is_keystroke_method("clipboard (wl-copy)"));
        assert!(!is_keystroke_method("clipboard (xclip/xsel)"));
    }

    #[test]
    fn test_chain_synthesizes_keystrokes() {
        assert!(!chain_synthesizes_keystrokes(["clipboard (wl-copy)"]));
        assert!(!chain_synthesizes_keystrokes([
            "clipboard (wl-copy)",
            "clipboard (xclip/xsel)"
        ]));
        assert!(!chain_synthesizes_keystrokes(std::iter::empty::<&str>()));
        assert!(chain_synthesizes_keystrokes([
            "paste (clipboard + keystroke)"
        ]));
        assert!(chain_synthesizes_keystrokes([
            "wtype",
            "dotool",
            "clipboard (wl-copy)"
        ]));
    }

    #[test]
    fn test_find_ydotool_socket_returns_none_when_no_socket() {
        // In a test environment there should be no ydotoold running, so this
        // should return None (no socket file exists at any candidate path).
        // This primarily verifies the function does not panic.
        let _result = find_ydotool_socket();
    }

    #[test]
    fn test_find_ydotool_socket_respects_env_override() {
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join(".ydotool_socket");

        // Create a real Unix socket so metadata().file_type().is_socket() is true
        let _listener = UnixListener::bind(&sock_path).unwrap();

        // Temporarily set YDOTOOL_SOCKET to point at our test socket.
        // This is inherently racy in multi-threaded test runs, but it's the
        // simplest way to exercise the env-var priority path.
        std::env::set_var("YDOTOOL_SOCKET", &sock_path);
        let result = find_ydotool_socket();
        std::env::remove_var("YDOTOOL_SOCKET");

        assert_eq!(result, Some(sock_path));
    }

    #[test]
    fn test_sanitize_urgency_valid() {
        assert_eq!(sanitize_urgency("low"), "low");
        assert_eq!(sanitize_urgency("normal"), "normal");
        assert_eq!(sanitize_urgency("critical"), "critical");
    }

    #[test]
    fn test_sanitize_urgency_invalid_falls_back_to_normal() {
        assert_eq!(sanitize_urgency(""), "normal");
        assert_eq!(sanitize_urgency("LOW"), "normal");
        assert_eq!(sanitize_urgency("urgent"), "normal");
        assert_eq!(sanitize_urgency("--rm -rf /"), "normal");
    }
}
