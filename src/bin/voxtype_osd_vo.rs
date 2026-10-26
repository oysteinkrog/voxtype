//! `voxtype-osd-vo`: the `swiss` dictation popup on the vo GPU overlay
//! framework.
//!
//! It reads the same inputs as `voxtype-osd-gtk4`: audio level frames from
//! the daemon's socket, and the live preview, status, state and suppression
//! files in the runtime directory. It draws the same look: the header with
//! time and word count, the filled waveform that freezes with its history
//! after the recording stops, the live words in up to five lines, and the
//! status line with its progress bar.
//!
//! The process stays up for the daemon's lifetime. Between dictations it
//! holds no window and no GPU device: each dictation opens a fresh overlay,
//! which draws its first frames on the CPU while the GPU starts, and closes
//! it when the daemon goes idle.
//!
//! `--offline DIR` renders a scripted dictation to PNG frames instead, for
//! screenshots and checks without a daemon or a display.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use clap::Parser;
use vo::scene::display::ShapeKind;
use vo::scene::element::{Element, TextStyle};
use vo::scene::geom::{Color, Size};
use vo::scene::taffy::{AlignItems, AlignSelf};
use vo::scene::text::TextLayout;
use vo::scene::timed::{Progress, Wave};
use vo::scene::{Prop, Transition};
use vo::{
    App, Cx, FontRegistry, Input, InputMode, LogDiagnostics, Options, OverlaySpec, Placement,
};
use vo_core::spring::Spring;
use vo_core::words::{Word, WordModel};

use voxtype::audio::levels::{AudioFrame, FRAME_BYTES, FRAME_HZ};
use voxtype::config::{Config as VoxtypeConfig, HotkeyConfig};
use voxtype::osd::config::{OsdConfig, OsdPosition};
use voxtype::osd::ipc::resolve_socket_path;
use voxtype::osd::plasma::{self, PlasmaTheme};
use voxtype::osd::swiss::{
    self, smooth_levels, stop_hint, tick_level, wave_hues, wave_stops, StatusLine,
};
use voxtype::osd::visual::Color as OsdColor;

/// Counts live heap bytes, for the own-heap figure in the diagnostics.
struct Counting;

static HEAP: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(by: usize) {
    let now = HEAP.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: forwards to the system allocator and only adds bookkeeping.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        HEAP.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new) };
        if !p.is_null() {
            HEAP.fetch_sub(layout.size(), Ordering::Relaxed);
            grow(new);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn heap() -> usize {
    HEAP.load(Ordering::Relaxed)
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1048576.0
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "voxtype-osd-vo",
    version,
    about = "Voxtype dictation popup (swiss look) on the vo GPU overlay framework"
)]
struct Args {
    /// Path to the voxtype config file. Defaults to
    /// `~/.config/voxtype/config.toml`. The `[osd]` and `[hotkey]` sections
    /// are read.
    #[arg(long, env = "VOXTYPE_CONFIG")]
    config: Option<PathBuf>,

    /// Path to the audio-frame Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    #[arg(long, env = "VOXTYPE_OSD_SOCKET")]
    socket: Option<PathBuf>,

    /// Seconds to wait between reconnect attempts when the daemon is down.
    #[arg(long, default_value = "1.0", env = "VOXTYPE_OSD_RECONNECT_SECS")]
    reconnect_secs: f32,

    /// Panel width in logical pixels.
    #[arg(long, env = "VOXTYPE_OSD_WIDTH")]
    width_px: Option<u32>,

    /// Margin from the screen edge in logical pixels, for corner positions.
    #[arg(long, env = "VOXTYPE_OSD_MARGIN")]
    margin_px: Option<u32>,

    /// Visual gain applied to audio samples before drawing the waveform.
    #[arg(long, env = "VOXTYPE_OSD_GAIN")]
    waveform_gain: Option<f32>,

    /// Accepted for compatibility with the GTK4 frontend's flags and ignored:
    /// this frontend always draws the swiss look.
    #[arg(long, env = "VOXTYPE_OSD_THEME", hide = true)]
    theme: Option<String>,

    /// Accepted for compatibility with the GTK4 frontend's flags and ignored.
    #[arg(long, env = "VOXTYPE_OSD_HEIGHT", hide = true)]
    height_px: Option<u32>,

    /// Accepted for compatibility with the GTK4 frontend's flags and ignored.
    #[arg(long, env = "VOXTYPE_OSD_LOG_EVERY", hide = true)]
    log_every: Option<u32>,

    /// Accepted for compatibility with the GTK4 frontend's flags and ignored.
    #[arg(long, env = "VOXTYPE_OSD_PEAK_DECAY", hide = true)]
    peak_decay_db_per_sec: Option<f32>,

    /// Draw the first frames on the GPU only. By default they are drawn on
    /// the CPU while the GPU starts, which puts the popup on screen sooner.
    #[arg(long, env = "VOXTYPE_OSD_VO_GPU_FIRST_FRAME")]
    gpu_first_frame: bool,

    /// Print startup timing, memory and frame counts to stderr.
    #[arg(long, env = "VOXTYPE_OSD_VO_DIAG")]
    diag: bool,

    /// Render a scripted dictation to PNG frames in this directory and exit.
    /// Needs a GPU but no display and no daemon.
    #[arg(long, value_name = "DIR")]
    offline: Option<PathBuf>,

    /// Frames per second for `--offline`.
    #[arg(long, default_value = "30")]
    fps: f64,
}

/// Silence on the audio socket longer than this, with no recording and no
/// final text in progress, closes the popup. The GTK4 frontend's value.
const IDLE_TIMEOUT_SECS: f64 = 0.15;

/// Audio held back so 10 to 40 ms capture batches scroll smoothly, as in
/// `osd::ipc::FrameRing`.
const PLAYOUT_DELAY_SECS: f64 = 0.05;

/// How often the runtime files are read. About 10 Hz is plenty for text.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A status file older than this is left over from a crash.
const STATUS_STALE: Duration = Duration::from_secs(30);

/// Font sizes in points, as the GTK4 swiss CSS gives them.
const CAPTION_PT: f32 = 8.5;
const CAPTION_SMALL_PT: f32 = 7.0;
const VALUE_PT: f32 = 11.0;
const STATUS_PT: f32 = 9.0;

/// Fonts in the registry, by id.
const SANS: u32 = 0;
const MONO: u32 = 1;

// Layout of the panel, in logical pixels. Mirrors the GTK4 swiss CSS.
const PAD_TOP: f32 = 16.0;
const PAD_X: f32 = 22.0;
const PAD_BOTTOM: f32 = 12.0;
const BORDER: f32 = 1.0;
const RADIUS: f32 = 3.0;
const ICON: f32 = 14.0;
const WAVE_MARGIN: f32 = 10.0;
const TEXT_MARGIN_BOTTOM: f32 = 4.0;
const RULE_MARGIN_TOP: f32 = 12.0;
const STATUS_MARGIN_TOP: f32 = 10.0;
const DOT: f32 = 7.0;

/// Line height for a font size: Noto Sans and most UI fonts set about 1.36
/// em, as Pango does.
fn line_height(px: f32) -> f32 {
    (px * 1.36).round()
}

/// Pixels per point from the font DPI GTK uses (`gtk-xft-dpi` in
/// `~/.config/gtk-4.0/settings.ini`, which Plasma writes from its font DPI
/// setting), so text is as large as in the GTK4 popup. 96 DPI without it.
fn px_per_pt() -> f32 {
    let dpi = dirs::config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("gtk-4.0/settings.ini")).ok())
        .and_then(|s| {
            s.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == "gtk-xft-dpi")
                .and_then(|(_, v)| v.trim().parse::<f32>().ok())
        })
        .map(|v| v / 1024.0)
        .filter(|dpi| (48.0..=480.0).contains(dpi))
        .unwrap_or(96.0);
    dpi / 72.0
}

fn color(c: OsdColor) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

/// Everything the view needs that does not change during a dictation.
#[derive(Clone)]
struct Look {
    width: f32,
    /// Font sizes in pixels.
    caption_px: f32,
    caption_small_px: f32,
    value_px: f32,
    status_px: f32,
    text_px: f32,
    text_line: f32,
    gain: f64,
    /// Seconds of audio across the waveform.
    window: f64,
    background: Color,
    foreground: Color,
    inactive: Color,
    accent: Color,
    border: Color,
    /// Waveform colours while recording: left, centre, right.
    hues: (Color, Color, Color),
    /// "tap Right Ctrl to stop", or empty.
    stop_hint: String,
}

impl Look {
    fn new(cfg: &OsdConfig, theme: &PlasmaTheme, stop_hint: String) -> Look {
        let k = px_per_pt();
        let text_px = cfg.preview_font_pt.unwrap_or(swiss::TEXT_PT) * k;
        let mut background = color(theme.background);
        background.a = cfg.opacity.clamp(0.0, 1.0);
        let (l, c, r) = wave_hues(theme.accent);
        Look {
            width: cfg.width_px as f32,
            caption_px: CAPTION_PT * k,
            caption_small_px: CAPTION_SMALL_PT * k,
            value_px: VALUE_PT * k,
            status_px: STATUS_PT * k,
            text_px,
            text_line: line_height(text_px),
            gain: cfg.waveform_gain as f64,
            window: cfg.waveform_window_secs.max(0.1) as f64,
            background,
            foreground: color(theme.foreground),
            inactive: color(theme.inactive),
            accent: color(theme.accent),
            border: color(theme.border),
            hues: (color(l), color(c), color(r)),
            stop_hint,
        }
    }

    /// Width of the transcript: the panel less padding and border.
    fn text_width(&self) -> f32 {
        self.width - swiss::PANEL_INSET_PX as f32
    }

    fn text_style(&self) -> TextStyle {
        TextStyle::new(SANS, self.text_px, self.text_line).tracking(-0.2)
    }

    fn header_height(&self) -> f32 {
        let stat = line_height(self.value_px) + 2.0 + line_height(self.caption_small_px);
        stat.max(line_height(self.caption_px)).max(ICON)
    }

    /// The panel's height with `lines` transcript lines.
    fn panel_height(&self, lines: u32) -> f32 {
        let status = line_height(self.status_px).max(DOT);
        2.0 * BORDER
            + PAD_TOP
            + self.header_height()
            + 2.0 * WAVE_MARGIN
            + swiss::WAVE_HEIGHT_PX as f32
            + lines as f32 * self.text_line
            + TEXT_MARGIN_BOTTOM
            + RULE_MARGIN_TOP
            + 1.0
            + STATUS_MARGIN_TOP
            + status
            + PAD_BOTTOM
    }

    /// The surface: room for the panel at its tallest. The panel grows
    /// inside it, so the surface never resizes; the rest is transparent and
    /// click-through.
    fn surface(&self) -> Size {
        Size::new(
            self.width.ceil(),
            self.panel_height(swiss::MAX_LINES as u32).ceil() + 2.0,
        )
    }
}

/// What the runtime files say, read about ten times a second.
#[derive(Clone, Debug, Default, PartialEq)]
struct Files {
    /// The daemon's state file says a recording runs.
    recording: bool,
    /// The status file is there and fresh: a model loads or the final text
    /// is being made.
    finishing: bool,
    /// `voxtype record start --no-osd`.
    suppressed: bool,
    /// The live preview text, trimmed. Empty when there is none.
    preview: String,
    /// The status file's fraction line, if it parses.
    fraction: Option<f64>,
    /// The status file's text line.
    status: String,
}

#[derive(Clone, Debug)]
enum Msg {
    /// Frames read from the socket in one go.
    Audio(Vec<AudioFrame>),
    Files(Files),
    /// Time to see whether the daemon went idle.
    Check,
}

/// The popup for one dictation. It closes itself when the daemon goes idle.
struct Popup {
    look: Look,
    measure: vo::ShapedLayout,
    /// Raw levels, oldest first, at most the waveform window plus a little.
    levels: VecDeque<f64>,
    last_seq: Option<u32>,
    /// `display(i) = anchor + PLAYOUT_DELAY + i / FRAME_HZ` for the i-th
    /// frame since the waveform started.
    anchor: f64,
    received: u64,
    /// The smoothed levels and the time the newest reaches the right edge.
    wave: Arc<[f32]>,
    newest_at: f64,
    /// When the popup opened and when the last frame came.
    shown_at: f64,
    last_audio: Option<f64>,
    files: Files,
    /// "00:12", frozen while the final text is made.
    clock: String,
    word_count: usize,
    model: WordModel,
    /// The words on screen: the tail of the transcript.
    shown: Vec<Word>,
    /// Transcript lines the text area has room for. Grows, never shrinks.
    lines: u32,
    check: Option<vo::TimerId>,
    closing: bool,
}

impl Popup {
    fn new(look: Look, fonts: &FontRegistry) -> Popup {
        Popup {
            look,
            measure: vo::ShapedLayout::new(fonts.clone()),
            levels: VecDeque::new(),
            last_seq: None,
            anchor: 0.0,
            received: 0,
            wave: Arc::from([]),
            newest_at: 0.0,
            shown_at: 0.0,
            last_audio: None,
            files: Files::default(),
            clock: "00:00".into(),
            word_count: 0,
            model: WordModel::new(),
            shown: Vec::new(),
            lines: 1,
            check: None,
            closing: false,
        }
    }

    fn status(&self) -> StatusLine {
        StatusLine::new(&self.files.status, &self.look.stop_hint)
    }

    fn audio(&mut self, frames: Vec<AudioFrame>, t: f64) {
        self.last_audio = Some(t);
        let window = (self.look.window * FRAME_HZ as f64).ceil() as usize + 12;
        let mut added = 0u64;
        for f in frames {
            if self
                .last_seq
                .is_some_and(|last| f.seq != last.wrapping_add(1))
            {
                // The daemon's silence pump keeps frames coming after the mic
                // stops, with its own counter: keep the waveform frozen.
                if f.is_pump_silence() {
                    continue;
                }
                // A new recording or a reconnect: start the waveform again.
                self.levels.clear();
                self.received = 0;
            }
            self.last_seq = Some(f.seq);
            self.levels
                .push_back(tick_level(f.max.max(-f.min) as f64, self.look.gain));
            if self.levels.len() > window {
                self.levels.pop_front();
            }
            added += 1;
        }
        if added == 0 {
            return;
        }
        let step = 1.0 / FRAME_HZ as f64;
        let before = self.received;
        if before == 0 {
            self.anchor = t;
        } else {
            // After an underrun, resume where the display stopped rather than
            // jump to the late data.
            let shown_last = self.anchor + PLAYOUT_DELAY_SECS + (before - 1) as f64 * step;
            if t > shown_last {
                self.anchor = t - PLAYOUT_DELAY_SECS - (before - 1) as f64 * step;
            }
        }
        self.received += added;
        self.newest_at = self.anchor + PLAYOUT_DELAY_SECS + (self.received - 1) as f64 * step;
        let raw: Vec<f64> = self.levels.iter().copied().collect();
        self.wave = smooth_levels(&raw).into_iter().map(|l| l as f32).collect();
        // The clock stops at the end of the recording: frames during the
        // final text come from the silence pump.
        let status = self.status();
        if !status.finishing {
            let secs = (t - self.shown_at).max(0.0) as u64;
            self.clock = format!("{:02}:{:02}", secs / 60, secs % 60);
        }
    }

    fn text(&mut self, preview: &str) {
        if preview.is_empty() {
            // The daemon removes the file at the end; keep the last words.
            return;
        }
        self.word_count = preview.split_whitespace().count();
        self.model.update(preview);
        let words = self.model.words();
        let style = self.look.text_style();
        let width = Some(self.look.text_width());
        let lines_of = |shown: &[Word]| {
            let text = shown
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let h = self.measure.layout(&text, &style, width).size.h;
            (h / style.line_height).round() as u32
        };
        let tail = |start: usize| {
            let mut shown = words[start..].to_vec();
            if start > 0 {
                if let Some(first) = shown.first_mut() {
                    first.text = format!("…{}", first.text);
                }
            }
            shown
        };
        // Drop words from the front until the rest fits.
        let mut start = words.len().saturating_sub(120);
        let mut shown = tail(start);
        while lines_of(&shown) > swiss::MAX_LINES as u32 && start + 1 < words.len() {
            start += 1;
            shown = tail(start);
        }
        self.lines = self
            .lines
            .max(lines_of(&shown).clamp(1, swiss::MAX_LINES as u32));
        self.shown = shown;
    }

    fn idle(&self, t: f64) -> bool {
        let stalled = self.last_audio.is_none_or(|a| t - a > IDLE_TIMEOUT_SECS);
        (stalled && !self.files.finishing && !self.files.recording) || self.files.suppressed
    }

    fn arm_check(&mut self, cx: &mut Cx<Msg>) {
        if let Some(id) = self.check.take() {
            cx.cancel(id);
        }
        self.check = Some(cx.after(IDLE_TIMEOUT_SECS + 0.01, Msg::Check));
    }
}

impl App for Popup {
    type Msg = Msg;

    fn init(&mut self, cx: &mut Cx<Msg>) {
        self.shown_at = cx.t();
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        if self.closing {
            return;
        }
        let t = cx.t();
        match msg {
            Msg::Audio(frames) => {
                self.audio(frames, t);
                self.arm_check(cx);
            }
            Msg::Files(files) => {
                let preview_changed = files.preview != self.files.preview;
                self.files = files;
                if preview_changed {
                    let preview = self.files.preview.clone();
                    self.text(&preview);
                }
                self.arm_check(cx);
            }
            Msg::Check => self.check = None,
        }
        if self.idle(t) {
            tracing::info!(
                recording = self.files.recording,
                status = self.files.finishing,
                suppressed = self.files.suppressed,
                since_audio = self.last_audio.map(|a| t - a),
                "closing: the daemon is idle"
            );
            self.closing = true;
            cx.exit();
        }
    }

    fn view(&self, _: &vo::Tokens) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.panel())
    }
}

impl Popup {
    fn panel(&self) -> Element<Msg> {
        let l = &self.look;
        let status = self.status();
        Element::panel("panel")
            .width(l.width)
            .pad_edges(
                PAD_TOP + BORDER,
                PAD_X + BORDER,
                PAD_BOTTOM + BORDER,
                PAD_X + BORDER,
            )
            .radius(RADIUS)
            .fill(l.background)
            .stroke(l.border, BORDER)
            .motion(Prop::Size, Spring::SNAP)
            .motion(Prop::Position, Spring::SNAP)
            .child(self.header())
            .child(self.waveform(&status))
            .child(self.transcript())
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .height(1.0)
                    .fill(l.border)
                    .margin_edges(RULE_MARGIN_TOP, 0.0, 0.0, 0.0),
            )
            .child(self.status_row(&status))
    }

    fn header(&self) -> Element<Msg> {
        let l = &self.look;
        let caption = |key: &'static str, text: &str, px: f32, tracking: f32| {
            Element::label(key, text)
                .text_style(TextStyle::new(MONO, px, line_height(px)).tracking(tracking))
                .color(l.inactive)
        };
        let stat = |key: &'static str, value: &str, name: &'static str| {
            Element::column(key)
                .gap(2.0)
                .child(
                    Element::label("value", value)
                        .text_style(TextStyle::new(MONO, l.value_px, line_height(l.value_px)))
                        .color(l.foreground),
                )
                .child(caption("caption", name, l.caption_small_px, 1.5))
        };
        // A microphone from shapes: the capsule and its stand.
        let mic = Element::column("mic")
            .size(ICON, ICON)
            .align_items(AlignItems::CENTER)
            .gap(1.5)
            .child(
                Element::shape("body", ShapeKind::Capsule)
                    .size(6.0, 9.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("stand", ShapeKind::RRect)
                    .size(1.5, 2.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("base", ShapeKind::RRect)
                    .size(7.0, 1.5)
                    .fill(l.accent),
            );
        Element::row("header")
            .align_items(AlignItems::START)
            .child(mic)
            .child(
                caption("title", "VOICE DICTATION", l.caption_px, 2.0)
                    .margin_edges(0.0, 0.0, 0.0, 8.0),
            )
            .child(Element::row("spacer").grow(1.0))
            .child(stat("time", &self.clock, "DURATION"))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .width(1.0)
                    .fill(l.border)
                    .margin_edges(0.0, 16.0, 0.0, 16.0)
                    .layout(|s| s.align_self = Some(AlignSelf::STRETCH)),
            )
            .child(stat("words", &self.word_count.to_string(), "WORDS"))
    }

    fn waveform(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        // Once the recording stops the frozen waveform turns grey.
        let (left, center, right) = if status.finishing {
            (l.inactive, l.inactive, l.inactive)
        } else {
            l.hues
        };
        let to_vo = |c: Color| OsdColor::rgba(c.r, c.g, c.b, c.a);
        let stops = wave_stops(to_vo(left), to_vo(center), to_vo(right))
            .into_iter()
            .map(|(p, c, a)| (p, Color::rgba(c.r, c.g, c.b, a)))
            .collect();
        let wave = Wave {
            levels: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
            window: l.window as f32,
            stops,
            min_half: 0.5,
            baseline: Some(l.border),
        };
        Element::wave("wave", wave)
            .height(swiss::WAVE_HEIGHT_PX as f32)
            .margin_edges(WAVE_MARGIN, 0.0, WAVE_MARGIN, 0.0)
    }

    fn transcript(&self) -> Element<Msg> {
        let l = &self.look;
        Element::column("text")
            .height(self.lines as f32 * l.text_line)
            .margin_edges(0.0, 0.0, TEXT_MARGIN_BOTTOM, 0.0)
            .motion(Prop::Size, Spring::SNAP)
            .child(
                Element::live_text("words", &self.shown)
                    .text_style(l.text_style())
                    .color(l.foreground)
                    .motion(Prop::Opacity, Spring::SNAP)
                    .motion(Prop::Offset, Spring::SNAP)
                    .word_enter(Transition::fade().offset(0.0, 3.0)),
            )
    }

    fn status_row(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        let mono = |px: f32| TextStyle::new(MONO, px, line_height(px));
        // A filled square while listening, a hollow one while finishing.
        let dot = if status.finishing {
            Element::shape("dot", ShapeKind::RRect)
                .size(DOT - 1.0, DOT - 1.0)
                .margin(0.5)
                .fill(Color::TRANSPARENT)
                .stroke(l.accent, 1.0)
        } else {
            Element::shape("dot", ShapeKind::RRect)
                .size(DOT, DOT)
                .fill(l.accent)
        };
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_MARGIN_TOP, 0.0, 0.0, 0.0)
            .child(dot)
            .child(
                Element::label("state", status.state.as_str())
                    .text_style(mono(l.status_px))
                    .color(l.accent)
                    .margin_edges(0.0, 0.0, 0.0, 10.0),
            )
            .child(
                Element::label("detail", status.detail.as_str())
                    .text_style(mono(l.status_px).clamp_lines(1))
                    .color(l.inactive)
                    .shrink(1.0)
                    .min_width(0.0),
            );
        if status.shows_progress() {
            let progress = Progress {
                fraction: self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32),
                track: l.border,
                bar: l.accent,
                since: self.shown_at,
                period: 1.6,
            };
            row = row.child(
                Element::progress("progress", progress)
                    .height(2.0)
                    .grow(1.0)
                    .margin_edges(0.0, 0.0, 0.0, 16.0),
            );
        }
        row
    }
}

/// Load `[osd]` and `[hotkey]` from the voxtype config, with defaults for
/// anything missing or unreadable, as the GTK4 frontend does.
fn load_config(explicit: Option<&Path>) -> (OsdConfig, HotkeyConfig) {
    #[derive(serde::Deserialize, Default)]
    struct Partial {
        #[serde(default)]
        osd: Option<OsdConfig>,
        #[serde(default)]
        hotkey: Option<HotkeyConfig>,
    }
    let partial = explicit
        .map(Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str::<Partial>(&s).ok())
        .unwrap_or_default();
    (
        partial.osd.unwrap_or_default(),
        partial.hotkey.unwrap_or_default(),
    )
}

/// The fonts the look names, through fontconfig: the Plasma UI font at
/// medium weight for the transcript, and the fixed font for captions.
fn load_fonts(theme: &PlasmaTheme) -> anyhow::Result<FontRegistry> {
    let fonts = FontRegistry::new();
    let sans = theme
        .font_family
        .clone()
        .unwrap_or_else(|| "sans-serif".into());
    let mono = theme
        .mono_family
        .clone()
        .unwrap_or_else(|| "monospace".into());
    let id = fonts.add_system(&format!("{sans}:medium"))?;
    debug_assert_eq!(id, SANS);
    let id = fonts.add_system(&mono)?;
    debug_assert_eq!(id, MONO);
    Ok(fonts)
}

fn placement(cfg: &OsdConfig) -> Placement {
    use vo::Edges;
    let m = cfg.margin_px as i32;
    let corner = |edges: Edges| Placement::Anchored {
        edges,
        margin: [m, m, m, m],
    };
    match cfg.position {
        OsdPosition::BottomCenter | OsdPosition::TopCenter => Placement::FromTop {
            fraction: cfg.top_margin.clamp(0.0, 1.0),
        },
        OsdPosition::TopLeft => corner(Edges {
            top: true,
            left: true,
            ..Edges::default()
        }),
        OsdPosition::TopRight => corner(Edges::TOP_RIGHT),
        OsdPosition::BottomLeft => corner(Edges {
            bottom: true,
            left: true,
            ..Edges::default()
        }),
        OsdPosition::BottomRight => corner(Edges::BOTTOM_RIGHT),
    }
}

/// Messages from the reader threads, held while no popup is open.
#[derive(Default)]
struct Sink {
    proxy: Option<vo::Proxy<Msg>>,
    audio: Vec<Vec<AudioFrame>>,
    files: Files,
}

/// Most audio batches held while no popup is open: about a second.
const HELD_AUDIO: usize = 64;

impl Sink {
    /// Sends to the open popup, or keeps the message for the next one.
    fn audio(&mut self, frames: Vec<AudioFrame>) {
        if let Some(p) = &self.proxy {
            if p.send(Msg::Audio(frames.clone())) {
                return;
            }
            self.proxy = None;
        }
        self.audio.push(frames);
        if self.audio.len() > HELD_AUDIO {
            self.audio.remove(0);
        }
    }

    fn files(&mut self, files: Files) {
        self.files = files.clone();
        if let Some(p) = &self.proxy {
            if !p.send(Msg::Files(files)) {
                self.proxy = None;
            }
        }
    }

    /// Whether a popup should open now.
    fn wants_popup(&mut self) -> bool {
        if self.files.suppressed {
            self.audio.clear();
            return false;
        }
        !self.audio.is_empty() || self.files.recording || self.files.finishing
    }
}

type Shared = Arc<(Mutex<Sink>, Condvar)>;

/// Reads frames from the daemon's audio socket and hands each read on as one
/// batch. Reconnects when the daemon goes away.
fn read_audio(shared: Shared, socket: PathBuf, reconnect: Duration) {
    let mut buf = vec![0u8; FRAME_BYTES * 64];
    loop {
        if let Ok(mut stream) = UnixStream::connect(&socket) {
            tracing::info!("connected to {}", socket.display());
            let mut have = 0usize;
            loop {
                match stream.read(&mut buf[have..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => have += n,
                }
                let whole = have / FRAME_BYTES * FRAME_BYTES;
                if whole == 0 {
                    continue;
                }
                let frames: Vec<AudioFrame> = buf[..whole]
                    .chunks_exact(FRAME_BYTES)
                    .map(|c| AudioFrame::from_bytes(c.try_into().expect("chunk of a frame")))
                    .collect();
                buf.copy_within(whole..have, 0);
                have -= whole;
                let (lock, cv) = &*shared;
                lock.lock().unwrap().audio(frames);
                cv.notify_one();
            }
            tracing::info!("audio socket closed");
        }
        std::thread::sleep(reconnect);
    }
}

fn read_files() -> Files {
    let dir = VoxtypeConfig::runtime_dir();
    let status_path = VoxtypeConfig::live_status_path();
    let finishing = std::fs::metadata(&status_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < STATUS_STALE);
    let status = std::fs::read_to_string(&status_path).unwrap_or_default();
    let (fraction, text) = status.split_once('\n').unwrap_or(("", ""));
    Files {
        recording: std::fs::read_to_string(dir.join("state"))
            .is_ok_and(|s| matches!(s.trim(), "recording" | "streaming")),
        finishing,
        suppressed: dir.join("osd_suppressed").exists(),
        preview: std::fs::read_to_string(VoxtypeConfig::live_preview_path())
            .unwrap_or_default()
            .trim()
            .to_string(),
        fraction: fraction.trim().parse::<f64>().ok(),
        status: text.trim().to_string(),
    }
}

/// Reads the runtime files and passes on each change.
fn poll_files(shared: Shared) {
    let mut last = None;
    loop {
        let files = read_files();
        if last.as_ref() != Some(&files) {
            last = Some(files.clone());
            let (lock, cv) = &*shared;
            lock.lock().unwrap().files(files);
            cv.notify_one();
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let started = std::time::Instant::now();
    let args = Args::parse();
    let (mut cfg, hotkey) = load_config(args.config.as_deref());
    if let Some(w) = args.width_px {
        cfg.width_px = w;
    }
    if let Some(m) = args.margin_px {
        cfg.margin_px = m;
    }
    if let Some(g) = args.waveform_gain {
        cfg.waveform_gain = g;
    }
    let theme = plasma::load();
    let look = Look::new(&cfg, &theme, stop_hint(&hotkey));
    let fonts = load_fonts(&theme)?;

    if let Some(dir) = &args.offline {
        return offline(look, fonts, dir, args.fps);
    }

    let shared: Shared = Arc::new((Mutex::new(Sink::default()), Condvar::new()));
    let socket = resolve_socket_path(args.socket.clone());
    let reconnect = Duration::from_secs_f32(args.reconnect_secs.max(0.05));
    tracing::info!(
        "voxtype-osd-vo starting; socket={:?} width={}",
        socket,
        cfg.width_px
    );
    {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("voxtype-osd-audio".into())
            .spawn(move || read_audio(shared, socket, reconnect))?;
    }
    {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("voxtype-osd-files".into())
            .spawn(move || poll_files(shared))?;
    }

    let spec = OverlaySpec {
        size: look.surface(),
        placement: placement(&cfg),
        input: InputMode::None,
        app_id: "voxtype-osd".into(),
        ..OverlaySpec::default()
    };
    loop {
        // Wait for a dictation, then hand everything held so far to a new
        // popup and route later messages straight to it.
        let inbox = {
            let (lock, cv) = &*shared;
            let mut sink = cv
                .wait_while(lock.lock().unwrap(), |s| !s.wants_popup())
                .unwrap();
            let (proxy, inbox) = vo::channel();
            // Audio first: frames that come before the daemon's state file
            // says "recording" must not let the popup close at once.
            for frames in sink.audio.drain(..) {
                proxy.send(Msg::Audio(frames));
            }
            proxy.send(Msg::Files(sink.files.clone()));
            sink.proxy = Some(proxy);
            inbox
        };
        tracing::info!("showing");
        if args.diag {
            eprintln!(
                "voxtype-osd-vo: showing at {:.4} s since main",
                started.elapsed().as_secs_f64()
            );
        }
        let options = Options {
            fonts: fonts.clone(),
            inbox: Some(inbox),
            cpu_first_frame: !args.gpu_first_frame,
            diagnostics: args
                .diag
                .then(|| Box::new(LogDiagnostics::new(Some(heap))) as Box<dyn vo::Diagnostics>),
            ..Options::default()
        };
        let result = vo::run_overlay_with(Popup::new(look.clone(), &fonts), spec.clone(), options);
        shared.0.lock().unwrap().proxy = None;
        if args.diag {
            eprintln!(
                "voxtype-osd-vo: heap now {:.2} MB, peak {:.2} MB",
                mb(heap()),
                mb(PEAK.load(Ordering::Relaxed))
            );
        }
        match result {
            Ok(()) => tracing::info!("hiding"),
            Err(e) => {
                // No display yet, or the compositor went away: try again on
                // the next dictation rather than exit, so the supervisor does
                // not give up on us.
                tracing::warn!("overlay failed: {e}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// A scripted dictation: model loading, words arriving, a pause, the stop,
/// the final text and the close.
fn scenario(look: &Look) -> vo::Scenario<Msg> {
    let mut inputs: Vec<(f64, Input<Msg>)> = Vec::new();
    let msg = |m: Msg| Input::Msg(m);
    let (start, stop, done) = (0.2, 6.6, 8.6);
    let pause = (3.4, 4.4);
    // Audio: two frames every 20 ms while recording, then the silence pump.
    let mut seq = 0u32;
    let mut t = start;
    while t < stop {
        let mut frames = Vec::new();
        for k in 0..2 {
            let ft = t + k as f64 * 0.01;
            let speaking = !(pause.0..pause.1).contains(&ft);
            let syllable = (ft * 2.0 * std::f64::consts::PI * 3.3).sin().abs();
            let phrase = 0.6 + 0.4 * (ft * 2.0 * std::f64::consts::PI * 0.45).sin();
            let amp = if speaking {
                0.004 + 0.28 * syllable * phrase
            } else {
                0.002
            } as f32;
            frames.push(AudioFrame {
                seq,
                min: -amp,
                max: amp,
                peak_dbfs: 20.0 * amp.log10(),
            });
            seq += 1;
        }
        inputs.push((t, msg(Msg::Audio(frames))));
        t += 0.02;
    }
    let mut pump = 0u32;
    let mut t = stop + 0.03;
    while t < done {
        let silent = AudioFrame {
            seq: pump,
            min: 0.0,
            max: 0.0,
            peak_dbfs: -120.0,
        };
        inputs.push((t, msg(Msg::Audio(vec![silent]))));
        pump += 1;
        t += 0.033;
    }
    // Text and status, as the file poller would see them.
    let sentence = "open the configuration file and set the popup theme to swiss so the dictation \
                    window shows the live words in the middle of the screen with the waveform above \
                    them then press the key again to stop and let the final pass fix the casing and \
                    the punctuation before the text is typed into the editor";
    let words: Vec<&str> = sentence.split(' ').collect();
    let speaking_time = (stop - start - 0.5) - (pause.1 - pause.0);
    let per_word = speaking_time / words.len() as f64;
    let mut files = Files {
        recording: true,
        ..Files::default()
    };
    let at = |t: f64, files: &Files, inputs: &mut Vec<(f64, Input<Msg>)>| {
        inputs.push((t, Input::Msg(Msg::Files(files.clone()))));
    };
    // The live model loads during the first second.
    files.finishing = true;
    files.status = "Loading: live text model, 0 s".into();
    at(start, &files, &mut inputs);
    files.fraction = Some(0.45);
    files.status = "Loading: live text model, 1 s".into();
    at(start + 0.5, &files, &mut inputs);
    files.finishing = false;
    files.fraction = None;
    files.status.clear();
    at(start + 1.0, &files, &mut inputs);
    let mut t = start + 0.6;
    for (i, _) in words.iter().enumerate() {
        if (pause.0..pause.1).contains(&t) {
            t = pause.1;
        }
        let mut text = words[..=i].join(" ");
        // A word still being spoken shows its start first.
        if words[i] == "configuration" {
            files.preview = format!("{} confi", words[..i].join(" "));
            at(t, &files, &mut inputs);
            t += per_word * 0.5;
        }
        if i + 1 == words.len() {
            text = text.trim().to_string();
        }
        files.preview = text;
        at(t, &files, &mut inputs);
        t += per_word;
    }
    // Stop: the final text is made, then pasted.
    files.recording = false;
    files.finishing = true;
    for (k, f) in [0.1, 0.35, 0.6, 0.85, 1.0].iter().enumerate() {
        files.fraction = Some(*f);
        files.status = format!("Finishing: {} s of audio left", 4 - k.min(4));
        at(stop + 0.05 + k as f64 * 0.35, &files, &mut inputs);
    }
    let mut fin: String = sentence
        .replace(" so the", ", so the")
        .replace(" then press", ". Then press");
    fin.replace_range(0..1, "O");
    fin.push('.');
    files.preview = fin;
    at(done - 0.4, &files, &mut inputs);
    files.finishing = false;
    files.fraction = None;
    files.status.clear();
    at(done, &files, &mut inputs);
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    vo::Scenario {
        size: look.surface(),
        scale: 1.0,
        duration: done + 1.0,
        inputs,
    }
}

fn offline(look: Look, fonts: FontRegistry, dir: &Path, fps: f64) -> anyhow::Result<()> {
    let scenario = scenario(&look);
    let options = Options {
        fonts: fonts.clone(),
        ..Options::default()
    };
    let n = vo::render_offline_with(Popup::new(look, &fonts), &scenario, fps, dir, options)
        .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
    eprintln!("voxtype-osd-vo: wrote {n} frames to {}", dir.display());
    eprintln!(
        "voxtype-osd-vo: heap peak {:.2} MB",
        mb(PEAK.load(Ordering::Relaxed))
    );
    Ok(())
}
