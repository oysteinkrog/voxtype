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
use std::collections::{HashSet, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use clap::Parser;
use vo::scene::display::ShapeKind;
use vo::scene::element::{Element, TextStyle};
use vo::scene::flow::bell::Bell;
use vo::scene::flow::margin::MarginMark;
use vo::scene::flow::overflow::Overflow;
use vo::scene::flow::spine::Spine;
use vo::scene::geom::{Color, Rect, Size};
use vo::scene::taffy::{AlignItems, AlignSelf};
use vo::scene::text::TextLayout;
use vo::scene::timed::{Progress, Wave};
use vo::scene::{Motion, Prop, Transition};
use vo::{
    App, Cx, FontRegistry, Input, InputMode, LogDiagnostics, Options, OverlaySpec, Placement,
};
use vo_core::signals::{SignalFrame, SignalsCell, WIRE_SAMPLE_RATE};
use vo_core::spring::Spring;
use vo_core::words::{Word, WordInfo, WordModel};

use voxtype::audio::features::AudioFeatures;
use voxtype::audio::levels::{AudioFrame, AudioReader, FRAME_HZ};
use voxtype::config::{Config as VoxtypeConfig, HotkeyConfig};
use voxtype::osd::config::{OsdAnchor, OsdConfig, OsdMotion, OsdPosition, OsdTheme};
use voxtype::osd::ipc::resolve_socket_path;
use voxtype::osd::look_step;
use voxtype::osd::outcome::Outcome;
use voxtype::osd::plasma::{self, PlasmaTheme};
use voxtype::osd::swiss::{
    self, smooth_levels, stop_hint, tick_level, wave_hues, wave_stops, StatusLine,
};
use voxtype::osd::visual::Color as OsdColor;

#[path = "osd_vo/swiss_look.rs"]
mod swiss_look;
#[path = "osd_vo/showcase.rs"]
mod showcase;
#[path = "osd_vo/type_look.rs"]
mod type_look;
#[path = "osd_vo/amber.rs"]
mod amber;
#[path = "osd_vo/radial.rs"]
mod radial;
#[path = "osd_vo/tron.rs"]
mod tron;
#[path = "osd_vo/phosphor.rs"]
mod phosphor;
#[path = "osd_vo/mission.rs"]
mod mission;
#[path = "osd_vo/palette.rs"]
mod palette;
#[path = "osd_vo/dotmatrix.rs"]
mod dotmatrix;
#[path = "osd_vo/kit.rs"]
mod kit;
#[path = "osd_vo/revision.rs"]
mod revision;
#[path = "osd_vo/letter_fx.rs"]
mod letter_fx;
#[path = "osd_vo/tally.rs"]
mod tally;
#[path = "osd_vo/limit.rs"]
mod limit;
#[path = "osd_vo/swissfi.rs"]
mod swissfi;
#[path = "osd_vo/scope.rs"]
mod scope;
#[path = "osd_vo/captions.rs"]
mod captions;
#[path = "osd_vo/karaoke.rs"]
mod karaoke;
#[path = "osd_vo/island.rs"]
mod island;
#[path = "osd_vo/envelope.rs"]
mod envelope;
#[path = "osd_vo/entry_latency.rs"]
mod entry_latency;
#[path = "osd_vo/jarvis.rs"]
mod jarvis;
#[path = "osd_vo/particles.rs"]
mod particles;
#[path = "osd_vo/cyberpunk.rs"]
mod cyberpunk;
#[path = "osd_vo/set_type.rs"]
mod set_type;
#[path = "osd_vo/orb.rs"]
mod orb;
#[path = "osd_vo/chrome.rs"]
mod chrome;
#[path = "osd_vo/glass.rs"]
mod glass;
#[path = "osd_vo/biolume.rs"]
mod biolume;
#[path = "osd_vo/aurora.rs"]
mod aurora;
#[path = "osd_vo/hologram.rs"]
mod hologram;
#[path = "osd_vo/river.rs"]
mod river;
#[path = "osd_vo/wall.rs"]
mod wall;
#[path = "osd_vo/replay.rs"]
mod replay;
#[path = "osd_vo/wild_motion.rs"]
mod wild_motion;
use wild_motion::WildMotion;
#[path = "osd_vo/moments.rs"]
mod moments;
#[path = "osd_vo/station/mod.rs"]
mod station;
#[path = "osd_vo/synthwave.rs"]
mod synthwave;
#[path = "osd_vo/mecha.rs"]
mod mecha;
#[path = "osd_vo/wild_space.rs"]
mod wild_space;
#[path = "osd_vo/defaults.rs"]
mod defaults;
#[path = "osd_vo/endings.rs"]
mod endings;
#[allow(unused_imports)]
use endings::RewindHide;
#[path = "osd_vo/sounding.rs"]
mod sounding;
#[path = "osd_vo/choice.rs"]
mod choice;
#[path = "osd_vo/hearing.rs"]
mod hearing;
#[path = "osd_vo/channels.rs"]
mod channels;
#[path = "osd_vo/glyph_paths.rs"]
mod glyph_paths;
#[path = "osd_vo/app_icon.rs"]
mod app_icon;
#[path = "osd_vo/lifecycle.rs"]
mod lifecycle;
#[path = "osd_vo/ruler.rs"]
mod ruler;
#[path = "osd_vo/model_signals.rs"]
mod model_signals;
#[path = "osd_vo/forming.rs"]
mod forming;
#[path = "osd_vo/display.rs"]
mod display;
#[path = "osd_vo/realtime.rs"]
mod realtime;
#[path = "osd_vo/correction_note.rs"]
mod correction_note;
// Parts of the keyboard edit link the edit layer (M7.3) draws from.
#[allow(dead_code)]
#[path = "osd_vo/edit_link.rs"]
mod edit_link;

#[path = "osd_vo/edit_focus.rs"]
mod edit_focus;

#[path = "osd_vo/edit.rs"]
mod edit;

#[path = "osd_vo/edit_site.rs"]
mod edit_site;

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

    /// What the centered positions place: `top` puts the popup's top edge
    /// at `top_margin` and grows it downward; `reading-line` puts the newest
    /// text line at `--reading-line` and grows the popup upward, so the line
    /// you read stays put. Also `[osd] anchor`.
    #[arg(long, env = "VOXTYPE_OSD_ANCHOR", value_parser = ["top", "reading-line", "center"])]
    anchor: Option<String>,

    /// Height of the newest text line as a fraction of the screen, with
    /// `--anchor reading-line`. 0.5 is the middle. Also `[osd] reading_line`.
    #[arg(long, env = "VOXTYPE_OSD_READING_LINE")]
    reading_line: Option<f32>,

    /// Visual gain applied to audio samples before drawing the waveform.
    #[arg(long, env = "VOXTYPE_OSD_GAIN")]
    waveform_gain: Option<f32>,

    /// Fade new words in instead of showing them at once. Also `[osd]
    /// word_fade = true`.
    #[arg(long, env = "VOXTYPE_OSD_WORD_FADE")]
    word_fade: bool,

    /// Show the active look's number and name, such as "3/27 Type", above
    /// the live popup's top-left corner. Also `[osd] look_badge = true`.
    #[arg(long, env = "VOXTYPE_OSD_LOOK_BADGE")]
    look_badge: bool,

    /// Real-time CPU priority for the popup while it shows: `true` (the
    /// default) or `false`. Also `[osd] realtime`.
    #[arg(long, env = "VOXTYPE_OSD_REALTIME")]
    realtime: Option<bool>,

    /// Mark a word as unsure when the engine's confidence in it is below
    /// this, 0 to 1 (default 0.45, about 1 % of real words). Also `[osd]
    /// doubt_below`.
    #[arg(long, env = "VOXTYPE_OSD_DOUBT_BELOW")]
    doubt_below: Option<f32>,

    /// Look to draw: `swiss` (default here), `showcase`, `type`, `amber`,
    /// `radial`, `tron`, `phosphor`, `mission`, `palette`, `dotmatrix`,
    /// `swissfi`, `scope`, `captions`, `karaoke`, `island`, `jarvis`,
    /// `particles`, `synthwave`, `mecha`, `cyberpunk`, `orb`, `chrome`,
    /// `glass`, `biolume`, `aurora`, `hologram` or `river`. Also `[osd]
    /// theme`. Other themes draw swiss.
    #[arg(long, env = "VOXTYPE_OSD_THEME")]
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

    /// Do not get the GPU ready at startup. By default the GPU device,
    /// shaders and glyphs are made in the background when the popup starts,
    /// so the first dictation shows as fast as later ones.
    #[arg(long, env = "VOXTYPE_OSD_VO_NO_PREWARM")]
    no_prewarm: bool,

    /// Print startup timing, memory and frame counts to stderr.
    #[arg(long, env = "VOXTYPE_OSD_VO_DIAG")]
    diag: bool,

    /// Render a scripted dictation to PNG frames in this directory and exit.
    /// Needs a GPU but no display and no daemon.
    #[arg(long, value_name = "DIR")]
    offline: Option<PathBuf>,

    /// Frames per second for `--offline`, `--envelope` and `--exit-check`.
    /// `--envelope` needs at least 30, the rate its limits are given at.
    #[arg(long, default_value = "30")]
    fps: f64,

    /// With `--offline` or `--envelope`: script a dictation that is
    /// cancelled at the pause, so the popup closes without a final text.
    #[arg(long)]
    cancel: bool,

    /// Check the scripted dictation of `--offline` (or `--moment NAME`)
    /// against the reading-line envelope and measure its GPU time per frame,
    /// then exit. Fails when either is breached. Needs a GPU but no display
    /// and no daemon.
    #[arg(long)]
    envelope: bool,

    /// Run the scripted dictation of `--offline` (or `--moment NAME`) on the
    /// CPU and print how long new words take to become readable: their
    /// entry animation's median, 90th percentile and longest time. Honours
    /// `--motion` and `VO_REDUCED_MOTION=1`. Needs no GPU, display or daemon.
    #[arg(long, hide = true)]
    entry_latency: bool,

    /// As `--entry-latency`, for the forming cue: how long after each voice
    /// onset it first shows, its glyph overlaps and its frames past the
    /// 150 ms quiet fade.
    #[arg(long, hide = true)]
    forming_latency: bool,

    /// Run the scripted dictation of `--offline` (or `--moment NAME`) with
    /// no PNGs and check that the popup's exit settles before vo's exit
    /// motion cap. Prints `exit-check: ok <look> settled <s> s after exit`,
    /// or `exit-check: FAIL ...` and exits 1. Honours `--cancel`, `--motion`
    /// and `VO_REDUCED_MOTION=1`. Needs no GPU, display or daemon.
    #[arg(long)]
    exit_check: bool,

    /// With `--offline`: print each surface's GPU time per frame (median,
    /// 95th percentile and worst) when the GPU has timestamps. Development.
    #[arg(long, requires = "offline", env = "VOXTYPE_OSD_VO_GPU_TIMES")]
    gpu_times: bool,

    /// Record each dictation's audio, voice features and runtime files to
    /// `DIR/<unix ms>.jsonl`, for `--preview --replay FILE`.
    #[arg(long, value_name = "DIR", env = "VOXTYPE_OSD_VO_RECORD")]
    record: Option<PathBuf>,

    /// Development: a wall of looks side by side on one surface, all fed the
    /// same dictation. With `--offline`, renders one loop of the demo.
    #[arg(long)]
    wall: bool,

    /// With `--wall`: the looks to show, comma separated, for example
    /// `swiss,showcase,type`. Default: every look the vo frontend draws.
    #[arg(long, value_name = "THEMES", requires = "wall", value_delimiter = ',')]
    wall_themes: Vec<String>,

    /// With `--wall`: cells per row.
    #[arg(long, value_name = "N", default_value = "3", requires = "wall")]
    wall_cols: usize,

    /// With `--wall`: `demo` loops a scripted dictation and a cancelled
    /// one; `live` follows the daemon, so you can dictate.
    #[arg(long, value_name = "SCRIPT", default_value = "demo", requires = "wall")]
    wall_script: String,

    /// With `--wall`: the layout size in logical pixels, `WxH`. Live, the
    /// wall covers the whole output; this is its size until the compositor
    /// says, and the size of an `--offline` render.
    #[arg(
        long,
        value_name = "WxH",
        default_value = "1800x1000",
        requires = "wall"
    )]
    wall_size: String,

    /// Motion features, comma separated; wins over `[osd] motion`. `look`
    /// (the default) is each look's own set and `none` turns it off. Any
    /// feature name replaces the set (for example `onset-ink`); `no-<name>`
    /// takes one name out of it (for example `no-fold-under`). Unknown names
    /// are ignored. `--station` exhibits keep their own features.
    #[arg(
        long,
        value_name = "NAMES",
        value_delimiter = ',',
        env = "VOXTYPE_OSD_VO_MOTION"
    )]
    motion: Vec<String>,

    /// With `--offline` or `--exit-check`: run one scripted moment instead
    /// of the demo dictation, for example `revision-burst`, `cancel` or
    /// `look-switch`. `--moment list` prints them all.
    #[arg(long, value_name = "MOMENT")]
    moment: Option<String>,

    /// Development: play one scripted moment (for example `dictation` or
    /// `long-silence`) live in the real popup surface, print a line per
    /// frame (with `VO_DIAG_FRAMES=1`) and the frame diagnostics, then exit.
    /// Needs a display: scripts/osd-vo-budget.sh runs it in a nested KWin.
    #[arg(long, value_name = "MOMENT")]
    live_moment: Option<String>,

    /// Development: the motion station, every motion exhibit as a labelled
    /// popup tile, page by page. With `--offline`, renders one page.
    #[arg(long)]
    station: bool,

    /// With `--station`: `live` follows the daemon (tiles loop their moments
    /// between dictations), `demo` only loops the moments. Default: live when
    /// the daemon runs, else demo.
    #[arg(long, value_name = "SCRIPT", requires = "station")]
    station_script: Option<String>,

    /// With `--station`: only these exhibits, comma separated, by MO id
    /// (`MO-20`), number (`20`) or name.
    #[arg(long, value_name = "IDS", requires = "station", value_delimiter = ',')]
    station_only: Vec<String>,

    /// With `--station`: hold this page (from 1) instead of turning pages.
    #[arg(long, value_name = "N", requires = "station")]
    station_page: Option<usize>,

    /// With `--station`: seconds each page shows.
    #[arg(long, value_name = "SECS", default_value = "30", requires = "station")]
    station_dwell: f64,

    /// With `--station`: tiles per row.
    #[arg(long, value_name = "N", default_value = "4", requires = "station")]
    station_cols: usize,

    /// With `--station`: rows of tiles per page.
    #[arg(long, value_name = "N", default_value = "3", requires = "station")]
    station_rows: usize,

    /// With `--station`: the layout size in logical pixels, `WxH`.
    #[arg(
        long,
        value_name = "WxH",
        default_value = "2560x1440",
        requires = "station"
    )]
    station_size: String,

    /// With `--station`: every tile in its reduced-motion form.
    #[arg(long, requires = "station")]
    station_reduced: bool,

    /// With `--station`: print the exhibits page by page, with their looks,
    /// features and moments, and exit.
    #[arg(long, requires = "station")]
    station_list: bool,
}

/// Silence on the audio socket longer than this, with no recording and no
/// final text in progress, closes the popup. The GTK4 frontend's value.
const IDLE_TIMEOUT_SECS: f64 = 0.15;

/// After a cancel or a failure the popup stays this long, so a look can
/// show it.
const CANCEL_LINGER_SECS: f64 = 0.6;

/// The daemon says how a dictation ended just after it goes idle. Until
/// then, or this long, the popup does not call it "Stopped".
const OUTCOME_GRACE_SECS: f64 = 0.1;

/// Audio held back so 10 to 40 ms capture batches scroll smoothly, as in
/// `osd::ipc::FrameRing`.
const PLAYOUT_DELAY_SECS: f64 = 0.05;

/// A capture frame whose peak stays under this is room tone (RD-11): the
/// hearing features call voice anything above about -45 dBFS.
const ROOM_TONE_DBFS: f64 = -45.0;

/// On room tone the popup draws at the ambient cap once the last louder
/// frame is this old, so the release of a word's peak scrolls at full rate.
const ROOM_TONE_AFTER_LOUD_SECS: f64 = 0.3;

/// The frame pace on room tone (RD-14), in frames a second. Half the ambient
/// cap (`vo_core::ambient::AMBIENT_FPS`, 30): the room-tone waveform is near
/// flat, and idle loops step at 30, 15 or fewer steps a second, so a 30-step
/// loop moves two steps a frame, evenly. 15 also divides both 60 and 75 Hz,
/// so every room-tone frame stays on screen for the same number of vblanks
/// (20 would alternate on a 75 Hz output). Idle-open CPU on the measure
/// build drops from about 2.3 to 5 % of one core at 30 to about 1.3 to 2.5 %.
const ROOM_TONE_FPS: f64 = 15.0;

/// ... and once the last change of words, state or look is this old, so
/// event motion (near still within 600 ms, MOTION-PLAN rule 8) and the
/// popup's opening keep the full rate.
const ROOM_TONE_AFTER_EVENT_SECS: f64 = 0.6;

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
/// Seconds the words of a new line wait while a springing panel opens for
/// it. The box opens over about 0.2 s; by now it has room for the line.
const ROOM_SECS: f64 = 0.08;
/// How long the status row names a look picked with the look chord.
const LOOK_NAME_SECS: f64 = 1.2;
const RULE_MARGIN_TOP: f32 = 12.0;
const STATUS_MARGIN_TOP: f32 = 10.0;
const DOT: f32 = 7.0;

/// Pixels per point as GTK4 sets them on Wayland: 96 DPI times the text
/// scale, so text is as large as in the GTK4 popup. GTK takes the font DPI
/// from the settings portal there, not from `gtk-xft-dpi` in settings.ini.
/// The microphone GTK shows: `audio-input-microphone-symbolic` from the GTK icon theme,
/// registered as a path. `None` when no plain SVG of it is found.
fn load_mic_icon(fonts: &FontRegistry) -> Option<(u32, Rect)> {
    let svg = icon_theme_names()
        .iter()
        .flat_map(|theme| icon_dirs().into_iter().map(move |dir| dir.join(theme)))
        .flat_map(|theme| {
            [
                "devices/16",
                "devices/22",
                "symbolic/devices",
                "16x16/devices",
                "scalable/devices",
            ]
            .map(|sub| theme.join(sub).join("audio-input-microphone-symbolic.svg"))
        })
        .find_map(|path| std::fs::read_to_string(path).ok())?;
    let (view, d) = svg_icon_path(&svg)?;
    let path = vo::scene::path::kurbo::BezPath::from_svg(&d).ok()?;
    Some((fonts.add_path(path), view))
}

/// The GTK icon theme, then the usual fallbacks.
fn icon_theme_names() -> Vec<String> {
    let config = dirs::config_dir().map(|d| d.join("gtk-4.0/settings.ini"));
    let gtk = config
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|ini| {
            ini.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == "gtk-icon-theme-name").then(|| v.trim().to_string())
            })
        });
    gtk.into_iter()
        .chain(["breeze", "Adwaita", "hicolor"].map(String::from))
        .collect()
}

/// Icon theme folders in lookup order: the user's, then each of `XDG_DATA_DIRS`.
fn icon_dirs() -> Vec<std::path::PathBuf> {
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs::data_dir()
        .into_iter()
        .chain(std::env::split_paths(&data_dirs))
        .map(|d| d.join("icons"))
        .collect()
}

/// The viewBox and the joined path data of a symbolic icon. Only plain icons qualify: paths
/// alone, with no transforms, so drawing `d` in the viewBox draws what GTK draws.
fn svg_icon_path(svg: &str) -> Option<(Rect, String)> {
    let shapes = [
        "<circle",
        "<rect",
        "<ellipse",
        "<polygon",
        "<polyline",
        "<line",
        "<use",
    ];
    if svg.contains("transform") || shapes.iter().any(|s| svg.contains(s)) {
        return None;
    }
    let attr = |tag: &str, name: &str| {
        let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(tag[at..].split('"').next()?.to_string())
    };
    let view: Vec<f32> = attr(svg, "viewBox")?
        .split([' ', ','])
        .filter(|v| !v.is_empty())
        .map(|v| v.parse().ok())
        .collect::<Option<_>>()?;
    let [x, y, w, h] = view[..] else { return None };
    let d: Vec<String> = svg
        .split("<path")
        .skip(1)
        .filter_map(|tag| attr(tag.split('>').next()?, "d"))
        .collect();
    (!d.is_empty() && w > 0.0 && h > 0.0).then(|| (Rect::new(x, y, w, h), d.join(" ")))
}

fn px_per_pt() -> f32 {
    96.0 / 72.0 * text_scaling_factor()
}

/// `org.gnome.desktop.interface text-scaling-factor` from the settings
/// portal, which GTK4 reads on Wayland; 1.0 when there is no portal or no
/// usable value.
fn text_scaling_factor() -> f32 {
    use zbus::zvariant::Value;
    let read = async {
        let conn = zbus::Connection::session().await.ok()?;
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &("org.gnome.desktop.interface", "text-scaling-factor"),
            )
            .await
            .ok()?;
        let body = reply.body();
        let mut value: Value = body.deserialize().ok()?;
        // Older portals wrap the value in one more variant.
        while let Value::Value(inner) = value {
            value = *inner;
        }
        match value {
            Value::F64(f) => Some(f as f32),
            _ => None,
        }
    };
    let factor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| {
            rt.block_on(async { tokio::time::timeout(Duration::from_millis(500), read).await })
                .ok()
                .flatten()
        });
    factor.filter(|f| (0.5..=3.0).contains(f)).unwrap_or(1.0)
}

/// A font's line height at `px` as Pango sets it: its ascent plus descent,
/// rounded up.
fn line_of(fonts: &FontRegistry, font: u32, px: f32) -> f32 {
    fonts.line_height(font, px).unwrap_or((px * 1.36).round())
}

fn color(c: OsdColor) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

/// A centred position: `bottom-center` or `top-center`.
fn centred(cfg: &OsdConfig) -> bool {
    matches!(
        cfg.position,
        OsdPosition::BottomCenter | OsdPosition::TopCenter
    )
}

/// The live surface's layout (`Look::live_frame`).
struct LiveFrame {
    /// Room above and left of the look.
    top: f32,
    left: f32,
    /// The badge strip's top and the badge's left edge.
    badge_top: f32,
    badge_left: f32,
    /// The height the centre anchor puts on the reading line.
    line: f32,
    size: Size,
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
    /// For line heights: each font's own, as Pango gives them.
    fonts: FontRegistry,
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
    /// New words fade in; otherwise they appear at once, as in GTK.
    word_fade: bool,
    /// The live popup names its look in a strip above the panel.
    badge: bool,
    /// `[osd] keyboard_edit`: the badge strip is there for the edit badge,
    /// so the popup keeps its size when an edit starts (R53).
    edit_badge: bool,
    /// `[osd] doubt_below`: a word under this confidence is drawn unsure.
    doubt_below: f32,
    /// `Swiss`, `Showcase` or `Type`; anything else draws swiss.
    theme: OsdTheme,
    /// The icon theme's microphone as a registered path and its viewBox, or `None` to draw
    /// one from shapes.
    mic: Option<(u32, Rect)>,
    /// With `anchor = "reading-line"`: the newest text line's height on the
    /// output, as a fraction. `None` places the popup by its top edge.
    reading_line: Option<f32>,
    /// `anchor = "center"` with a centred position: each look's panel is
    /// centred on the reading line, and the badge strip reaches up to the
    /// tallest look's panel (`live_frame`).
    centre_anchor: bool,
    /// A centred position: the badge lines up with the left edge of a
    /// `width` wide panel in every look.
    centred: bool,
    /// `frame_reach`, worked out once.
    frame_reach: Arc<OnceLock<f32>>,
    /// Motion features asked for by name (`--motion`, `[osd] motion`, a
    /// station exhibit), `no-<name>` entries included.
    features: Arc<[String]>,
    /// The look's own motion set (`defaults::defaults`) is on too, minus
    /// `no-` entries. Off in `Look::new`, so a station tile keeps exactly
    /// its exhibit's features; `set_motion` turns it on.
    look_set: bool,
    /// Features switched off while a keyboard edit is pending or under way
    /// (`edit_site::quiet_list`); empty otherwise.
    quiet: Arc<[&'static str]>,
}

/// Letter spacing of a full newest line under type's margin bell (MO-31).
const TYPE_BELL_TIGHT: f32 = 0.97;

/// The line motion features of a look on its transcript (MO-31 to MO-34).
trait LineMotion {
    fn line_motion(self, look: &Look) -> Self;
}

impl<M> LineMotion for Element<M> {
    fn line_motion(self, look: &Look) -> Element<M> {
        let words = self.overflow(look.overflow());
        let words = match look.margin_mark() {
            Some(m) => words.margin_mark(m),
            None => words,
        };
        let words = match look.spine() {
            Some(s) => words.overflow_spine(s),
            None => words,
        };
        match look.bell() {
            Some(b) => words.margin_bell(b),
            None => words,
        }
    }
}

impl Look {
    fn new(cfg: &OsdConfig, theme: &PlasmaTheme, stop_hint: String, fonts: &FontRegistry) -> Look {
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
            text_line: line_of(fonts, SANS, text_px),
            gain: cfg.waveform_gain as f64,
            fonts: fonts.clone(),
            window: cfg.waveform_window_secs.max(0.1) as f64,
            background,
            foreground: color(theme.foreground),
            inactive: color(theme.inactive),
            accent: color(theme.accent),
            border: color(theme.border),
            hues: (color(l), color(c), color(r)),
            stop_hint,
            word_fade: cfg.word_fade,
            badge: cfg.look_badge,
            edit_badge: cfg.keyboard_edit,
            doubt_below: cfg.doubt_below,
            theme: cfg.theme,
            mic: load_mic_icon(fonts),
            reading_line: (cfg.anchor == OsdAnchor::ReadingLine)
                .then(|| cfg.reading_line.clamp(0.0, 1.0)),
            centre_anchor: centred(cfg) && cfg.anchor == OsdAnchor::Center,
            centred: centred(cfg),
            frame_reach: Arc::default(),
            features: Arc::from([]),
            look_set: false,
            quiet: Arc::from([]),
        }
    }

    /// Takes the resolved motion setting: `[osd] motion`, or `--motion` /
    /// `VOXTYPE_OSD_VO_MOTION` over it.
    fn set_motion(&mut self, m: &OsdMotion) {
        let (names, look_set) = defaults::resolve(m);
        self.features = names.into();
        self.look_set = look_set;
        // A look's size can depend on its features.
        self.frame_reach = Arc::default();
    }

    fn asked(&self, name: &str) -> bool {
        self.features.iter().any(|f| f == name)
    }

    fn removed(&self, name: &str) -> bool {
        self.features
            .iter()
            .any(|f| f.strip_prefix("no-") == Some(name))
    }

    /// Asked for, or in the current look's set and not taken out. Reads
    /// `self.theme` now, so a look switch mid-show answers the new look.
    fn asked_or_set(&self, name: &str) -> bool {
        if self.quiet.contains(&name) {
            return false;
        }
        self.asked(name)
            || (self.look_set
                && defaults::in_set(self.theme, name)
                && !self.removed(name))
    }

    /// Whether the motion feature `name` is switched on. Read by the looks
    /// that have motion features.
    #[allow(dead_code)]
    fn on(&self, name: &str) -> bool {
        if self.quiet.contains(&name) {
            return false;
        }
        // MO-75: `look-choice` also turns on the motions this look picks.
        self.asked_or_set(name)
            || (self.asked_or_set(choice::LOOK_CHOICE)
                && choice::picks(self.theme).contains(&name)
                && !self.removed(name))
    }

    /// How the top line leaves when the text overflows (MO-33). With `fold-under` it
    /// folds under about its baseline, or in the flat Swiss looks thins away; under
    /// reduced motion, and without the feature, it scrolls up and fades.
    fn overflow(&self) -> Overflow {
        if !self.on("fold-under") || self.on("reduced-motion") || kit::reduced() {
            return Overflow::Scroll;
        }
        match self.theme {
            OsdTheme::Default | OsdTheme::Swiss | OsdTheme::Swissfi => Overflow::THIN,
            _ => Overflow::FOLD,
        }
    }

    /// With `margin-mark`, a bar beside the newest line in the accent colour that splits
    /// when the text wraps (MO-32); under reduced motion it moves to the new line at once.
    fn margin_mark(&self) -> Option<MarginMark> {
        if !self.on("margin-mark") {
            return None;
        }
        let mark = MarginMark::new(self.accent);
        Some(if self.on("reduced-motion") {
            MarginMark { travel: Spring::critical_to_98(0.001), fade: 0.001, ..mark }
        } else {
            mark
        })
    }

    /// With `margin-bell`, the newest line tightens as it nears the edge and relaxes on
    /// the wrap (MO-31); type and phosphor add a 3 px carriage return. Under reduced motion
    /// nothing moves.
    fn bell(&self) -> Option<Bell> {
        if !self.on("margin-bell") || self.on("reduced-motion") {
            return None;
        }
        Some(match self.theme {
            // At 4 % a short word on type's newest line read 5 % tight at the default
            // width (envelope tracking 95 %, the limit is 5 %); 3 % stays inside.
            OsdTheme::Type => Bell {
                tight: TYPE_BELL_TIGHT,
                ..Bell::DEFAULT.carriage(3.0)
            },
            OsdTheme::Phosphor => Bell::DEFAULT.carriage(3.0),
            _ => Bell::DEFAULT,
        })
    }

    /// With `overflow-spine`, each line that leaves the top leaves a tick in the accent
    /// colour along the text's top edge (MO-34); under reduced motion the tick appears at
    /// once.
    fn spine(&self) -> Option<Spine> {
        if !self.on("overflow-spine") {
            return None;
        }
        let spine = Spine { y: 1.5, ..Spine::new(self.accent) };
        Some(if self.on("reduced-motion") {
            Spine { hold: 0.0, close: 0.0, ..spine }
        } else {
            spine
        })
    }

    /// The centre of the newest text line in logical pixels below the
    /// surface's top edge, with `lines` lines of text and the look drawn
    /// unshifted. `None` for a look that does not say; it is then placed by
    /// its middle and never shifted.
    fn newest_line_centre(&self, lines: u32) -> Option<f32> {
        match self.theme {
            OsdTheme::Synthwave => Some(synthwave::newest_line_centre(self)),
            OsdTheme::Default | OsdTheme::Swiss => Some(
                BORDER
                    + PAD_TOP
                    + self.header_height()
                    + 2.0 * WAVE_MARGIN
                    + swiss::WAVE_HEIGHT_PX as f32
                    + (lines as f32 - 0.5) * self.text_line,
            ),
            OsdTheme::Mecha => Some(mecha::newest_line_centre(self)),
            OsdTheme::Cyberpunk => Some(cyberpunk::newest_line_centre()),
            OsdTheme::Showcase => showcase::newest_line_centre(self, lines),
            OsdTheme::Type => type_look::newest_line_centre(self, lines),
            OsdTheme::Jarvis => Some(jarvis::newest_line_centre(self, lines)),
            OsdTheme::Particles => Some(particles::newest_line_centre(self, lines)),
            OsdTheme::River => Some(river::newest_line_centre(self, lines)),
            OsdTheme::Hologram => Some(hologram::newest_line_centre(self, lines)),
            OsdTheme::Glass => Some(glass::newest_line_centre(self)),
            OsdTheme::Chrome => Some(chrome::newest_line_centre(self, lines)),
            OsdTheme::Orb => Some(orb::newest_line_centre(self, lines)),
            _ => None,
        }
    }

    /// With the reading-line anchor: how far down the look is drawn with
    /// `lines` lines, so that its newest line sits where a full look's
    /// newest line does. The look grows upward as lines are added.
    fn reading_shift(&self, lines: u32) -> Option<f32> {
        self.reading_line?;
        let max = self.max_lines();
        let full = self.newest_line_centre(max)?;
        Some((full - self.newest_line_centre(lines.clamp(1, max))?).max(0.0))
    }

    /// Width of the transcript: the panel less padding and border.
    fn text_width(&self) -> f32 {
        if self.theme == OsdTheme::Amber {
            return amber::text_width(self);
        }
        if self.theme == OsdTheme::Radial {
            return radial::text_width(self);
        }
        if self.theme == OsdTheme::Tron {
            return tron::text_width(self);
        }
        if self.theme == OsdTheme::Phosphor {
            return phosphor::text_width(self);
        }
        if self.theme == OsdTheme::Scope {
            return scope::text_width(self);
        }
        if self.theme == OsdTheme::Captions {
            return captions::text_width(self);
        }
        if self.theme == OsdTheme::Karaoke {
            return karaoke::text_width(self);
        }
        if self.theme == OsdTheme::Island {
            return island::text_width(self);
        }
        if self.theme == OsdTheme::Jarvis {
            return jarvis::text_width(self);
        }
        if self.theme == OsdTheme::Particles {
            return particles::text_width(self);
        }
        if self.theme == OsdTheme::Synthwave {
            return synthwave::text_width(self);
        }
        if self.theme == OsdTheme::Mecha {
            return mecha::text_width(self);
        }
        if self.theme == OsdTheme::Cyberpunk {
            return cyberpunk::text_width(self);
        }
        if self.theme == OsdTheme::Swissfi {
            return swissfi::text_width(self);
        }
        if self.theme == OsdTheme::Dotmatrix {
            return dotmatrix::text_width(self);
        }
        if self.theme == OsdTheme::Palette {
            return palette::text_width(self);
        }
        if self.theme == OsdTheme::Mission {
            return mission::text_width(self);
        }
        if self.theme == OsdTheme::River {
            return river::text_width(self);
        }
        if self.theme == OsdTheme::Hologram {
            return hologram::text_width(self);
        }
        if self.theme == OsdTheme::Glass {
            return glass::text_width(self);
        }
        if self.theme == OsdTheme::Chrome {
            return chrome::text_width(self);
        }
        if self.theme == OsdTheme::Orb {
            return orb::text_width(self);
        }
        self.width - swiss::PANEL_INSET_PX as f32
    }

    /// Transcript lines the look has room for.
    fn max_lines(&self) -> u32 {
        if self.theme == OsdTheme::Scope {
            return scope::MAX_LINES;
        }
        if self.theme == OsdTheme::Captions {
            return captions::MAX_LINES;
        }
        if self.theme == OsdTheme::Karaoke {
            return karaoke::MAX_LINES;
        }
        if self.theme == OsdTheme::Island {
            return island::MAX_LINES;
        }
        if self.theme == OsdTheme::Mecha {
            return mecha::MAX_LINES;
        }
        if self.theme == OsdTheme::Swissfi {
            return swissfi::max_lines();
        }
        if self.theme == OsdTheme::Dotmatrix {
            return dotmatrix::max_lines();
        }
        if self.theme == OsdTheme::Palette {
            return palette::max_lines();
        }
        if self.theme == OsdTheme::Mission {
            return mission::max_lines();
        }
        if self.theme == OsdTheme::River {
            return river::max_lines();
        }
        if self.theme == OsdTheme::Hologram {
            return hologram::max_lines();
        }
        if self.theme == OsdTheme::Glass {
            return glass::MAX_LINES;
        }
        if self.theme == OsdTheme::Chrome {
            return chrome::MAX_LINES;
        }
        if self.theme == OsdTheme::Orb {
            return orb::MAX_LINES;
        }
        swiss::MAX_LINES as u32
    }

    fn mono_line(&self, px: f32) -> f32 {
        line_of(&self.fonts, MONO, px)
    }

    /// The fixed font at `px` with its own line height.
    fn mono(&self, px: f32) -> TextStyle {
        TextStyle::new(MONO, px, self.mono_line(px))
    }

    /// Whether the "…" before cut text hangs in the left margin, so it never
    /// changes where the lines break. The other looks put it on the word.
    fn margin_ellipsis(&self) -> bool {
        matches!(
            self.theme,
            OsdTheme::Default
                | OsdTheme::Swiss
                | OsdTheme::Type
                | OsdTheme::Dotmatrix
                | OsdTheme::Palette
        )
    }

    fn text_style(&self) -> TextStyle {
        if self.theme == OsdTheme::Amber {
            return amber::text_style(self);
        }
        if self.theme == OsdTheme::Radial {
            return radial::text_style(self);
        }
        if self.theme == OsdTheme::Tron {
            return tron::text_style(self);
        }
        if self.theme == OsdTheme::Phosphor {
            return phosphor::text_style(self);
        }
        if self.theme == OsdTheme::Scope {
            return scope::text_style(self);
        }
        if self.theme == OsdTheme::Captions {
            return captions::text_style(self);
        }
        if self.theme == OsdTheme::Karaoke {
            return karaoke::text_style(self);
        }
        if self.theme == OsdTheme::Island {
            return island::text_style(self);
        }
        if self.theme == OsdTheme::Jarvis {
            return jarvis::text_style(self);
        }
        if self.theme == OsdTheme::Particles {
            return particles::text_style(self);
        }
        if self.theme == OsdTheme::Synthwave {
            return synthwave::text_style(self);
        }
        if self.theme == OsdTheme::Mecha {
            return mecha::text_style(self);
        }
        if self.theme == OsdTheme::Cyberpunk {
            return cyberpunk::text_style(self);
        }
        if self.theme == OsdTheme::Swissfi {
            return swissfi::text_style(self);
        }
        if self.theme == OsdTheme::Dotmatrix {
            return dotmatrix::text_style(self);
        }
        if self.theme == OsdTheme::Palette {
            return palette::text_style(self);
        }
        if self.theme == OsdTheme::Mission {
            return mission::text_style(self);
        }
        if self.theme == OsdTheme::River {
            return river::text_style(self);
        }
        if self.theme == OsdTheme::Hologram {
            return hologram::text_style(self);
        }
        if self.theme == OsdTheme::Glass {
            return glass::text_style(self);
        }
        if self.theme == OsdTheme::Chrome {
            return chrome::text_style(self);
        }
        if self.theme == OsdTheme::Orb {
            return orb::text_style(self);
        }
        TextStyle::new(SANS, self.text_px, self.text_line).tracking(-0.2)
    }

    fn header_height(&self) -> f32 {
        let stat = self.mono_line(self.value_px) + 2.0 + self.mono_line(self.caption_small_px);
        stat.max(self.mono_line(self.caption_px)).max(ICON)
    }

    /// The panel's height with `lines` transcript lines.
    fn panel_height(&self, lines: u32) -> f32 {
        let status = self.mono_line(self.status_px).max(DOT);
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
    /// inside it; the rest is transparent and click-through. Room for ink
    /// outside the panel is not counted here: vo's automatic margin
    /// (`SurfaceMargin::Auto`) adds it, and grows the surface while an
    /// effect needs more (EG-08).
    /// The live surface: `surface` plus the look badge's strip on top.
    fn live_surface(&self) -> Size {
        self.live_frame().size
    }

    /// Where the live surface puts the look and its badge. With the centre
    /// anchor every look's panel middle sits on the same line, and the badge
    /// sits just above the tallest look's panel; with a centred position it
    /// lines up with the left edge of a `width` wide panel. So the badge is
    /// on the same screen pixel in every look and through a whole dictation.
    fn live_frame(&self) -> LiveFrame {
        let s = self.surface();
        let badge = self.badge_h();
        let mid = self.panel_middle();
        let (top, badge_top) = if self.centre_anchor && badge > 0.0 {
            // The badge's strip ends where the tallest panel starts. The
            // line is on a whole pixel, so the host's rounding of the
            // surface's place moves no look's badge.
            let reach = self.frame_reach().ceil() + badge;
            let line = reach.max(mid.ceil());
            (line - mid, line - reach)
        } else {
            (badge, 0.0)
        };
        // An even width keeps the surface's left edge on a whole pixel.
        let mut w = s.w;
        if self.centred {
            if badge > 0.0 {
                w = w.max(self.width.ceil());
            }
            w = (w / 2.0).ceil() * 2.0;
        }
        LiveFrame {
            top,
            left: ((w - s.w) / 2.0).floor(),
            badge_top,
            badge_left: if self.centred {
                ((w - self.width) / 2.0).floor().max(0.0)
            } else {
                0.0
            },
            line: top + mid,
            size: Size::new(w, (top + s.h).ceil()),
        }
    }

    /// The height in the look's own surface, drawn unshifted, that the
    /// centre anchor puts on the reading line: the middle of the panel at
    /// its tallest. Room a look keeps only for ink in motion is left out.
    fn panel_middle(&self) -> f32 {
        match self.theme {
            OsdTheme::Radial => radial::panel_middle(self),
            OsdTheme::Particles => particles::panel_middle(self),
            _ => self.surface().h / 2.0,
        }
    }

    /// The top of the panel in the look's own surface: below the room some
    /// looks keep above it for their glow or sparks.
    fn panel_top(&self) -> f32 {
        match self.theme {
            OsdTheme::Radial => radial::panel_top(self),
            OsdTheme::Particles => particles::panel_top(self),
            OsdTheme::Showcase => voxtype::osd::showcase::MARGIN_PX,
            OsdTheme::Jarvis => jarvis::panel_top(),
            OsdTheme::Glass => glass::panel_top(self),
            OsdTheme::Chrome => chrome::panel_top(self),
            OsdTheme::Orb => orb::panel_top(self),
            _ => 0.0,
        }
    }

    /// How far above the reading line the tallest panel reaches, of all
    /// the looks the look chord steps through.
    fn frame_reach(&self) -> f32 {
        *self.frame_reach.get_or_init(|| {
            let mut look = self.clone();
            OsdTheme::VO_LOOKS
                .iter()
                .chain([&self.theme])
                .map(|&theme| {
                    look.theme = theme;
                    look.panel_middle() - look.panel_top()
                })
                .fold(0.0, f32::max)
        })
    }

    /// Height of the look badge's strip above the panel, or 0 without it.
    fn badge_h(&self) -> f32 {
        if self.badge || self.edit_badge {
            BADGE_H
        } else {
            0.0
        }
    }

    fn surface(&self) -> Size {
        if self.theme == OsdTheme::Amber {
            return Popup::amber_surface(self);
        }
        if self.theme == OsdTheme::Radial {
            return Popup::radial_surface(self);
        }
        if self.theme == OsdTheme::Tron {
            return Popup::tron_surface(self);
        }
        if self.theme == OsdTheme::Phosphor {
            return Popup::phosphor_surface(self);
        }
        if self.theme == OsdTheme::Scope {
            return Popup::scope_surface(self);
        }
        if self.theme == OsdTheme::Captions {
            return Popup::captions_surface(self);
        }
        if self.theme == OsdTheme::Karaoke {
            return Popup::karaoke_surface(self);
        }
        if self.theme == OsdTheme::Island {
            return Popup::island_surface(self);
        }
        if self.theme == OsdTheme::Jarvis {
            return Popup::jarvis_surface(self);
        }
        if self.theme == OsdTheme::Particles {
            return Popup::particles_surface(self);
        }
        if self.theme == OsdTheme::Showcase {
            return Popup::showcase_surface(self);
        }
        if self.theme == OsdTheme::Mission {
            return mission::surface(self);
        }
        if self.theme == OsdTheme::Palette {
            return palette::surface(self);
        }
        if self.theme == OsdTheme::Dotmatrix {
            return dotmatrix::surface(self);
        }
        if self.theme == OsdTheme::Swissfi {
            return swissfi::surface(self);
        }
        if self.theme == OsdTheme::Type {
            return Popup::type_surface(self);
        }
        if self.theme == OsdTheme::Cyberpunk {
            return Popup::cyber_surface(self);
        }
        if self.theme == OsdTheme::Synthwave {
            return Popup::synthwave_surface(self);
        }
        if self.theme == OsdTheme::Mecha {
            return mecha::surface(self);
        }
        if self.theme == OsdTheme::River {
            return river::surface(self);
        }
        if self.theme == OsdTheme::Hologram {
            return hologram::surface(self);
        }
        if self.theme == OsdTheme::Glass {
            return glass::surface(self);
        }
        if self.theme == OsdTheme::Chrome {
            return chrome::surface(self);
        }
        if self.theme == OsdTheme::Orb {
            return orb::surface(self);
        }
        Size::new(
            self.width.ceil(),
            self.panel_height(swiss::MAX_LINES as u32).ceil(),
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
    /// `preview.json`: the words with their timing and how settled they are.
    live: Option<voxtype::osd::live::LiveDoc>,
    /// The state file says "transcribing": the recording stopped and the
    /// final text is being made, before or without a status line.
    transcribing: bool,
    /// How the dictation ended, once the daemon says so. Older daemons do
    /// not write it.
    outcome: Option<Outcome>,
    /// `result.json`: what a typed or pasted dictation delivered.
    delivery: Option<voxtype::osd::outcome::Delivery>,
    /// `not-pasted`: why the text went to the clipboard only, when the
    /// target window was not confirmed.
    not_pasted: Option<String>,
    /// `osd-look`: the last press of the look chord.
    look_step: Option<look_step::Step>,
}

/// How a dictation ended, as a look shows it in the moment before the popup
/// closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    /// The final text was made (and, with a daemon that says so, delivered).
    /// The popup stays `lifecycle::DONE_LINGER_SECS`, or until the hand-off ends.
    Done,
    /// The recording stopped with no final pass and no word from the daemon.
    Stopped,
    /// The user cancelled. The popup stays `CANCEL_LINGER_SECS`.
    Cancelled,
    /// No speech was heard. The popup stays `CANCEL_LINGER_SECS`.
    Empty,
    /// Something failed. The popup stays `CANCEL_LINGER_SECS`.
    Failed,
}

impl Ending {
    /// The popup stays a moment so the look can show it (`Popup::linger_secs`).
    fn lingers(self) -> bool {
        self != Ending::Stopped
    }

    fn label(self) -> &'static str {
        match self {
            Ending::Done => "Done",
            Ending::Stopped => "Stopped",
            Ending::Cancelled => "Cancelled",
            Ending::Empty => "No speech",
            Ending::Failed => "Failed",
        }
    }
}

/// One frame's voice features, by its sequence number.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Voice {
    seq: u32,
    features: AudioFeatures,
    /// The frame's peak level in dBFS.
    peak_dbfs: f32,
    /// Samples since the recording started, at the end of the frame; 0 when
    /// the daemon does not say.
    clock: u64,
}

/// Puts the frames that carry a sample clock on the signal bus, as one batch
/// that arrived at scene time `t`. The frames come in clock order.
fn feed_signals(signals: &SignalsCell, voice: &[Voice], t: f64) {
    signals.push_batch(voice.iter().filter_map(Voice::signal_frame), t);
}

impl Voice {
    /// The frame for the scene's signal bus, as `SignalFrame::from_wire_v3`
    /// reads it off the socket. `None` without a sample clock: the bus
    /// places frames by audio time.
    fn signal_frame(&self) -> Option<SignalFrame> {
        if self.clock == 0 {
            return None;
        }
        let f = &self.features;
        let mut mel = [0.0; 24];
        for (m, &b) in mel.iter_mut().zip(&f.mel) {
            *m = b as f32 / 255.0;
        }
        Some(SignalFrame {
            clock: self.clock as f64 / WIRE_SAMPLE_RATE,
            rms_dbfs: f.rms_dbfs,
            peak_dbfs: self.peak_dbfs,
            f0_hz: f.f0_hz,
            voiced: f.voiced,
            centroid_hz: f.centroid_hz,
            flatness: f.flatness,
            onset: f.onset,
            mel,
        })
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Audio carries frames inline; boxing would add an allocation per capture batch
enum Msg {
    /// Frames read from the socket in one go, with their voice features when
    /// the daemon sends them. One message per capture batch, so the view is
    /// built once per batch.
    Audio(Vec<AudioFrame>, Vec<Voice>),
    Files(Files),
    /// Time to see whether the daemon went idle.
    Check,
    /// An ambient frame of the showcase look.
    Tick,
    /// The theme wall starts its cells afresh. A popup ignores it.
    Loop,
    /// A timer of the motion station. A popup ignores it.
    Station(Box<station::Cue>),
    /// `preview-signals.json` changed: the live model took a step. Kept
    /// apart from `Files`, so it never touches the text or the lifecycle.
    ModelSignals(Box<voxtype::osd::live::ModelSignals>),
}

/// The popup for one dictation. It closes itself when the daemon goes idle.
struct Popup {
    look: Look,
    /// The voice features on the scene's signal bus, for `Bands::Signals`,
    /// `Element::bind` and the shaders' `sig_*` functions.
    signals: SignalsCell,
    /// Where this dictation's messages are recorded (`--record`).
    recorder: Option<vo::record::Recorder>,
    measure: vo::ShapedLayout,
    /// Raw levels, oldest first, at most the waveform window plus a little.
    levels: VecDeque<f64>,
    /// Voice features of the frames in `levels`, oldest first: loudness,
    /// pitch, brightness, noisiness and mel bands, for effects.
    voice: VecDeque<Voice>,
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
    /// When a capture frame last peaked above room tone.
    loud_at: f64,
    /// When the runtime files last changed: words, state, look.
    event_at: f64,
    /// The frame pace for `App::frame_pace`, set after each update.
    pace: Option<f64>,
    files: Files,
    /// "00:12", frozen while the final text is made.
    clock: String,
    word_count: usize,
    model: WordModel,
    /// The words on screen: the tail of the transcript.
    shown: Vec<Word>,
    /// The first word on screen. It only moves forward, a whole line at a time.
    cut: usize,
    /// Transcript lines the text area has room for. Grows, never shrinks.
    lines: u32,
    check: Option<vo::TimerId>,
    closing: bool,
    /// In a cell of the theme wall: closing empties the cell rather than
    /// ending the app.
    in_wall: bool,
    /// The daemon has said a recording runs since the popup opened.
    was_recording: bool,
    /// The daemon has said it makes the final text since the popup opened.
    was_finishing: bool,
    /// When the daemon went idle after a recording.
    quiet_since: Option<f64>,
    /// When the dictation ended, as `ending()` first said.
    ended_at: Option<f64>,
    /// The linger after a cancel or a failure has its idle check.
    linger_armed: bool,
    /// What only the showcase look keeps.
    extra: showcase::Extra,
    /// What only the amber look keeps.
    amber: amber::State,
    /// What only the radial look keeps.
    radial: radial::State,
    /// What only the tron look keeps.
    tron: tron::State,
    /// What only the phosphor look keeps.
    phos: phosphor::State,
    /// What the looks after the mock-ups (kit.rs) keep.
    fam: kit::State,
    /// New words, revisions and the pace (tally.rs).
    tally: tally::State,
    /// When the text area last gained a line, and the lines it had before.
    grew: Option<(f64, u32)>,
    /// Karaoke: when its newest line began, and that line's first word.
    line_born: Option<(f64, vo_core::words::WordId)>,
    /// What only the scope look keeps.
    scope: scope::State,
    /// What only the captions look keeps.
    captions: captions::State,
    /// What only the karaoke look keeps.
    karaoke: karaoke::State,
    /// What only the island look keeps.
    island: island::State,
    /// What only the jarvis look keeps.
    jarvis: jarvis::Extra,
    /// What only the glass look keeps.
    glass: glass::State,
    /// The final pass that sets the type (MO-40).
    set_type: set_type::State,
    /// Ink, light and liquid letter effects (`osd_vo/letter_fx.rs`).
    letter_fx: letter_fx::State,
    /// What only the cyberpunk look keeps.
    cyber: cyberpunk::State,
    /// What only the synthwave look keeps.
    synth: synthwave::State,
    /// The loading word's times and fill (MO-54).
    endings: endings::State,
    /// Onsets and the sound each word came from (`osd_vo/sounding.rs`).
    heard: sounding::State,
    /// The depth stack, lattice and sound sculpture (`osd_vo/wild_space.rs`).
    wild: wild_space::State,
    /// What the popup heard before the words came (`osd_vo/hearing.rs`).
    hearing: hearing::State,
    /// Typographic channels on the words (`osd_vo/channels.rs`).
    channels: channels::State,
    /// The end of the dictation: the delivery and its hand-off.
    life: lifecycle::State,
    /// The variable font's voice readings (MO-81).
    breath: wild_motion::State,
    /// The live popup's config, for sizing and placing a look picked with
    /// the look chord. `None` elsewhere: the chord does nothing there.
    osd: Option<OsdConfig>,
    /// A scripted run (`--offline`, `--exit-check`): the look chord steps
    /// the look with no live config, and never saves it.
    scripted: bool,
    /// The first runtime files came: a look step after them is a new press.
    step_seen: bool,
    /// When the look chord last picked a look, to name it in the status row.
    picked_at: Option<f64>,
    /// Runs on a `live_spec` surface, which has room for the look badge.
    live_surface: bool,
    /// The live model's latest step: frame levels and the lookahead tail.
    model_signals: Option<voxtype::osd::live::ModelSignals>,
    /// What the model signal features keep between steps.
    signal_state: model_signals::State,
    /// The outcome of the newest spoken correction, while it shows.
    correction_note: correction_note::State,
    /// The keyboard edit link, in a live popup with `[osd] keyboard_edit`.
    edit: Option<edit_link::EditLink>,
    /// Keyboard focus for an edit (the focus proxy).
    focus: edit_focus::Focus,
    /// The shared edit layer and what it shows while the popup owns the
    /// text (`osd_vo/edit.rs`).
    edit_layer: edit::EditLayer,
    edit_view: Option<edit::EditView>,
    /// Words typed or changed by keyboard edits in this dictation, kept
    /// until the next recording: the looks underline them (plan 4.1).
    edit_marks: HashSet<vo_core::words::WordId>,
    /// Edits this popup has shown, for the hint.
    edit_entries: u32,
    /// An edit was handed over to be delivered (Enter): the text is final,
    /// so the quiet list stays on until the next recording.
    edit_sent: bool,
    /// The text the words were last built from: the preview, or after an
    /// edit the edited text and the speech since.
    source: String,
}

impl Popup {
    /// A popup for a scripted run: the look chord steps the look, unsaved.
    fn scripted(look: Look, fonts: &FontRegistry) -> Popup {
        Popup {
            scripted: true,
            ..Popup::new(look, fonts)
        }
    }

    fn new(look: Look, fonts: &FontRegistry) -> Popup {
        Popup {
            look,
            signals: SignalsCell::default(),
            model_signals: None,
            signal_state: model_signals::State::default(),
            correction_note: correction_note::State::default(),
            edit: None,
            focus: edit_focus::Focus::default(),
            edit_layer: edit::EditLayer::default(),
            edit_view: None,
            edit_marks: HashSet::new(),
            edit_entries: 0,
            edit_sent: false,
            source: String::new(),
            recorder: None,
            measure: vo::ShapedLayout::new(fonts.clone()),
            levels: VecDeque::new(),
            voice: VecDeque::new(),
            last_seq: None,
            anchor: 0.0,
            received: 0,
            wave: Arc::from([]),
            newest_at: 0.0,
            shown_at: 0.0,
            loud_at: f64::NEG_INFINITY,
            event_at: 0.0,
            pace: None,
            last_audio: None,
            files: Files::default(),
            clock: "00:00".into(),
            word_count: 0,
            model: WordModel::new(),
            shown: Vec::new(),
            cut: 0,
            lines: 1,
            check: None,
            closing: false,
            in_wall: false,
            was_recording: false,
            was_finishing: false,
            quiet_since: None,
            ended_at: None,
            linger_armed: false,
            extra: showcase::Extra::default(),
            amber: amber::State::default(),
            radial: radial::State::default(),
            tron: tron::State::default(),
            phos: phosphor::State::default(),
            fam: kit::State::default(),
            tally: tally::State::default(),
            grew: None,
            line_born: None,
            scope: scope::State::default(),
            captions: captions::State::default(),
            karaoke: karaoke::State::default(),
            island: island::State::default(),
            jarvis: jarvis::Extra::default(),
            glass: glass::State::default(),
            set_type: set_type::State::default(),
            letter_fx: letter_fx::State::default(),
            cyber: cyberpunk::State::default(),
            synth: synthwave::State::default(),
            endings: endings::State::default(),
            heard: sounding::State::default(),
            wild: wild_space::State::default(),
            hearing: hearing::State::default(),
            channels: channels::State::default(),
            life: lifecycle::State::default(),
            breath: wild_motion::State::default(),
            osd: None,
            scripted: false,
            step_seen: false,
            picked_at: None,
            live_surface: false,
        }
    }

    /// The daemon's spoken correction state, when it has one.
    fn correction_view(&self) -> Option<&voxtype::osd::correction::CorrectionView> {
        self.files.live.as_ref()?.correction.as_ref()
    }

    fn status(&self) -> StatusLine {
        if self
            .picked_at
            .is_some_and(|at| self.extra.now - at < LOOK_NAME_SECS)
        {
            return StatusLine {
                state: format!("{:?}", self.look.theme),
                detail: String::new(),
                loading: false,
                finishing: false,
            };
        }
        if let Some((state, detail)) =
            correction_note::status(self.correction_view(), &self.correction_note, self.extra.now)
        {
            return StatusLine {
                state,
                detail,
                loading: false,
                finishing: false,
            };
        }
        if let Some(ending) = self.ending() {
            // The daemon is idle and the popup is about to close: an empty
            // status would read as "Listening", as if a new recording began.
            // After a reported delivery the status says what happened: "Typed".
            let state = match self.delivered() {
                Some(d) => d.label().into(),
                None => ending.label().into(),
            };
            return StatusLine {
                state,
                detail: String::new(),
                loading: false,
                finishing: true,
            };
        }
        if self.files.status.is_empty() && self.files.transcribing {
            // Stopped, and the daemon has not yet written how far the final
            // text is: not "Listening" any more.
            return StatusLine::new("Finishing", "");
        }
        StatusLine::new(&self.files.status, &self.look.stop_hint)
    }

    /// A recording ran, has stopped, and nothing is being finished: the
    /// short time between the daemon going idle and the popup closing.
    fn ended(&self) -> bool {
        self.ending().is_some()
    }

    /// How the dictation ended, once it has; `None` while it records or the
    /// final text is made. Looks use this to tell a cancel from a normal end.
    fn ending(&self) -> Option<Ending> {
        if !self.was_recording || self.busy() {
            return None;
        }
        Some(match self.files.outcome {
            Some(Outcome::Done) => Ending::Done,
            Some(Outcome::Cancelled) => Ending::Cancelled,
            Some(Outcome::Empty) => Ending::Empty,
            Some(Outcome::Failed) => Ending::Failed,
            None if self.was_finishing || self.extra.finish_at.is_some() => Ending::Done,
            // The outcome may be on its way: a cancel is written just after
            // the state goes idle.
            None if self
                .quiet_since
                .is_none_or(|q| self.extra.now - q < OUTCOME_GRACE_SECS) =>
            {
                return None
            }
            None => Ending::Stopped,
        })
    }

    /// The frame pace at `t` (RD-11): `ROOM_TONE_FPS` while the popup records
    /// room tone with nothing else going on, else `None` (every vblank). The
    /// waveform keeps scrolling with its whole history and the clock keeps
    /// ticking; only the redraw rate drops.
    fn room_tone_pace(&self, t: f64) -> Option<f64> {
        let quiet = self.files.recording
            && !self.files.transcribing
            && self.files.status.is_empty()
            && !self.closing
            && t - self.loud_at >= ROOM_TONE_AFTER_LOUD_SECS
            && t - self.event_at >= ROOM_TONE_AFTER_EVENT_SECS;
        // The raymarched blob and orb cost more a frame; their slow idle
        // motion reads the same at a lower pace.
        let fps = match self.look.theme {
            OsdTheme::Chrome => chrome::ROOM_TONE_FPS,
            OsdTheme::Orb => orb::ROOM_TONE_FPS,
            _ => ROOM_TONE_FPS,
        };
        quiet.then_some(fps)
    }

    /// The daemon records or makes the final text.
    fn busy(&self) -> bool {
        self.files.recording || self.files.transcribing || !self.files.status.is_empty()
    }

    /// Seconds since the dictation ended, or `None` while it runs.
    fn ended_for(&self, t: f64) -> Option<f64> {
        self.ended_at.map(|at| t - at)
    }

    /// The progress bar shows while a model loads or the final text is made,
    /// not once the dictation has ended.
    fn shows_progress(&self, status: &StatusLine) -> bool {
        status.shows_progress() && !self.ended()
    }

    /// Keeps the voice features of the frames on screen, and puts them on the
    /// signal bus as arrived at `t`.
    fn voice(&mut self, voice: Vec<Voice>, t: f64) {
        feed_signals(&self.signals, &voice, t);
        let window = (self.look.window * FRAME_HZ as f64).ceil() as usize + 12;
        self.voice.extend(voice);
        while self.voice.len() > window {
            self.voice.pop_front();
        }
    }

    /// The newest frame's voice features, if the daemon sends them.
    #[allow(dead_code)]
    fn latest_voice(&self) -> Option<&AudioFeatures> {
        self.voice.back().map(|v| &v.features)
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
                self.voice.clear();
                self.received = 0;
            }
            self.last_seq = Some(f.seq);
            let amp = f.max.max(-f.min) as f64;
            if amp > 0.0 && 20.0 * amp.log10() > ROOM_TONE_DBFS {
                self.loud_at = t;
            }
            self.levels
                .push_back(tick_level(f.max.max(-f.min) as f64, self.look.gain));
            if self.levels.len() > window {
                self.levels.pop_front();
            }
            added += 1;
        }
        // The clock runs on every frame, the silence pump's too, until the
        // daemon says it is finishing, as in GTK.
        if !self.status().finishing {
            let secs = (t - self.shown_at).max(0.0) as u64;
            self.clock = format!("{:02}:{:02}", secs / 60, secs % 60);
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
    }

    /// Hands the daemon's word facts to the word model, when `preview.json`
    /// describes the text on screen. The two files are written one after the
    /// other, so either may be read first.
    fn word_info(&mut self) {
        let Some(doc) = self.files.live.as_ref() else {
            return;
        };
        if doc.text != self.files.preview {
            return;
        }
        let info: Vec<WordInfo> = doc
            .words
            .iter()
            .map(|w| WordInfo {
                start: w.start,
                end: w.end,
                conf: w.conf,
                settled: doc.rev.saturating_sub(w.since),
                changes: w.changes,
            })
            .collect();
        self.model.set_info(&info);
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
        let inline = !self.look.margin_ellipsis();
        let (cut, shown) = cut_lines(words, self.cut, self.look.max_lines(), inline, lines_of);
        self.cut = cut;
        self.lines = self
            .lines
            .max(lines_of(&shown).clamp(1, self.look.max_lines()));
        self.shown = shown;
    }

    /// While the popup owns the edit text, the buffer is the text source
    /// and the edit layer picks the window of lines the look shows
    /// (`osd_vo/edit.rs`). Returns false when it does not own the text.
    fn edit_text(&mut self, t: f64) -> bool {
        let owned = self.edit.as_ref().and_then(|l| {
            l.buffer()
                .filter(|_| l.phase().is_some_and(edit_link::Phase::owns_text))
        });
        let Some(buffer) = owned else {
            if self.edit_view.take().is_some() {
                self.edit_layer.reset();
                // Back to the source text, now the edited text.
                self.source.clear();
            }
            return false;
        };
        let text = buffer.text().to_string();
        if self.edit_view.is_none() {
            self.edit_entries += 1;
        }
        self.model.update(&text);
        let site = edit_site::site(self.look.theme);
        let text_box = site.text_box_in(self);
        let edit_lines = site.edit_lines_in(self);
        let view = self.edit_layer.view(
            buffer,
            &self.measure,
            text_box.style,
            text_box.width,
            edit_lines,
            t,
        );
        let window = view.start..view.start + view.text.len();
        self.shown = edit::window_words(self.model.words(), &text, window);
        self.word_count = self.model.words().len();
        // Grows upward by whole lines, never shrinks (R53).
        self.lines = self.lines.max(view.lines);
        self.edit_view = Some(view);
        self.mark_edits();
        true
    }

    /// The edit view's marks: the marked words in its window.
    fn mark_edits(&mut self) {
        let (Some(view), Some(buffer)) = (
            self.edit_view.as_mut(),
            self.edit.as_ref().and_then(|l| l.buffer()),
        ) else {
            return;
        };
        let window = view.start..view.start + view.text.len();
        view.decor.marks =
            edit::mark_ranges(self.model.words(), buffer.text(), window, &self.edit_marks);
    }

    /// R34: from a pending tap until resume or a cancelled tap, the look's
    /// quiet list is off. Enter is neither: after it hands the text over
    /// to be delivered, the list stays off until the next recording, so the
    /// final text does not take the look's live motion back.
    fn sync_quiet(&mut self) {
        use edit_link::Phase;
        let link = self.edit.as_ref();
        let phase = link.and_then(|l| l.phase());
        if phase == Some(Phase::Releasing) && link.is_some_and(|l| l.sending()) {
            self.edit_sent = true;
        }
        let quiet = self.edit_sent
            || phase.is_some_and(|p| {
                matches!(
                    p,
                    Phase::Pending | Phase::Settling | Phase::Editing | Phase::Detached
                )
            });
        // Made again each time: a look switched mid-edit has its own list.
        self.look.quiet = if quiet {
            edit_site::quiet_list(self.look.theme).into()
        } else {
            Arc::from([])
        };
    }

    /// The text the look shows when the popup does not own the edit text:
    /// the preview, or after an edit the edited text, then the preview's
    /// words after the freeze boundary.
    fn source_text(&self) -> String {
        let block = self
            .edit
            .as_ref()
            .and(self.files.live.as_ref())
            .and_then(|l| l.edit.as_ref())
            .filter(|b| !b.frozen.is_empty());
        let preview = &self.files.preview;
        // Handed back and not yet answered: the daemon's block still has the
        // text from before the edit, so the buffer stands in for it.
        let handed_back = self
            .edit
            .as_ref()
            .filter(|l| l.phase() == Some(edit_link::Phase::Releasing))
            .and_then(|l| l.buffer())
            .map(|b| b.text());
        match block {
            Some(b) => {
                let frozen = handed_back.unwrap_or(&b.frozen);
                let after = voxtype::osd::edit_doc::after_words(preview, b.raw_words).trim();
                if after.is_empty() {
                    frozen.to_string()
                } else {
                    format!("{} {after}", frozen.trim_end())
                }
            }
            None => preview.clone(),
        }
    }

    /// How many shown words have room in the text area now. In the looks
    /// whose panel springs open for a new line (type, mission), just after
    /// the area gains a line only the words that fit the old lines, for
    /// [`ROOM_SECS`], so a word does not draw outside its box.
    pub(crate) fn room(&self) -> usize {
        let all = self.shown.len();
        let Some((at, before)) = self.grew else {
            return all;
        };
        if self.still() || self.extra.now - at >= ROOM_SECS {
            return all;
        }
        let l = &self.look;
        let style = l.text_style();
        let fits = |n: usize| {
            let text = self.shown[..n]
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let h = self
                .measure
                .layout(&text, &style, Some(l.text_width()))
                .size
                .h;
            (h / style.line_height).round() as u32 <= before
        };
        (0..=all).rev().find(|&n| fits(n)).unwrap_or(0)
    }

    fn idle(&self, t: f64) -> bool {
        if self.files.suppressed {
            return true;
        }
        // An ending stays a moment, so the look can show it.
        let lingering = self.ending().is_some_and(Ending::lingers)
            && self.ended_for(t).is_some_and(|d| d < self.linger_secs());
        let stalled = self.last_audio.is_none_or(|a| t - a > IDLE_TIMEOUT_SECS);
        stalled
            && !lingering
            && !self.files.finishing
            && !self.files.recording
            && !self.files.transcribing
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
        self.event_at = cx.t();
        self.extra.now = cx.t();
        self.sync_showcase(cx.t());
        self.sync_scope(cx.t());
        self.sync_captions(cx.t());
        self.sync_karaoke(cx.t());
        self.sync_island(cx.t());
        self.sync_cyber(cx.t());
        self.arm_tick(cx);
        self.arm_scope_tick(cx);
        self.arm_captions_tick(cx);
        self.arm_karaoke_tick(cx);
        self.arm_island_tick(cx);
        self.arm_hologram_tick(cx);
        self.arm_cyber_tick(cx);
        self.arm_dotmatrix_tick(cx);
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        if self.closing {
            return;
        }
        if let Some(r) = &self.recorder {
            for rec in replay::Recorded::of(&msg) {
                let _ = r.record_at(cx.t(), &rec);
            }
        }
        let msg = match msg {
            Msg::ModelSignals(s) => {
                tracing::debug!(
                    step = s.step,
                    audio_s = s.audio_s,
                    frames = s.frames_total,
                    tail = %s.with_tail,
                    "model signals"
                );
                self.note_signals(&s, cx.t());
                self.model_signals = Some(*s);
                return;
            }
            m => m,
        };
        let t = cx.t();
        self.extra.now = t;
        // An ambient frame never closes the popup; only the daemon's
        // messages and the idle check do.
        let tick = matches!(msg, Msg::Tick);
        match msg {
            Msg::Audio(frames, voice) => {
                // A recording played back keeps frames and features apart.
                if !frames.is_empty() {
                    self.audio(frames, t);
                    self.arm_check(cx);
                }
                if !voice.is_empty() {
                    self.voice(voice, t);
                }
            }
            Msg::Files(files) => {
                let step = new_step(self.step_seen, self.files.look_step, files.look_step);
                self.step_seen = true;
                let preview_changed = files.preview != self.files.preview;
                let live_changed = files.live != self.files.live;
                if files != self.files {
                    self.event_at = t;
                }
                self.extra.files(&self.files, &files, t);
                if files.recording && !self.files.recording {
                    // A new recording: the last dictation's edit marks go,
                    // and its look's motion comes back.
                    self.edit_marks.clear();
                    self.edit_sent = false;
                }
                self.was_recording |= files.recording;
                self.was_finishing |= files.transcribing || (files.finishing && !files.recording);
                self.files = files;
                if self.busy() {
                    // A new recording in the same popup.
                    self.quiet_since = None;
                    self.ended_at = None;
                    self.linger_armed = false;
                } else if self.was_recording && self.quiet_since.is_none() {
                    self.quiet_since = Some(t);
                    // Looks again once the outcome has had time to come.
                    cx.after(OUTCOME_GRACE_SECS + 0.01, Msg::Check);
                }
                if let (true, Some(link)) = (live_changed, self.edit.as_mut()) {
                    link.sync(self.files.live.as_ref().and_then(|l| l.edit.as_ref()));
                    self.focus.follow(link, cx, t);
                }
                self.sync_quiet();
                let editing = self.edit_text(t);
                let source = self.source_text();
                if !editing && source != self.source {
                    self.source = source;
                    let source = self.source.clone();
                    let before = self.lines;
                    self.text(&source);
                    if self.lines > before {
                        self.grew = Some((t, before));
                        // A wake-up for the view once the new line has room.
                        cx.after(ROOM_SECS + 0.01, Msg::Loop);
                    }
                    if let Some(wait) = self.karaoke_new_line(t) {
                        cx.after(wait + 0.01, Msg::Loop);
                    }
                }
                if preview_changed || live_changed {
                    self.word_info();
                    self.note_doubtful(t);
                }
                if live_changed {
                    let view = self.files.live.as_ref().and_then(|l| l.correction.as_ref());
                    if let Some(wait) = self.correction_note.files(view, t) {
                        // Drawn again when the outcome leaves the status row.
                        cx.after(wait + 0.01, Msg::Loop);
                    }
                }
                if let Some(step) = step {
                    self.step_look(step.forward, t, cx);
                }
                self.arm_check(cx);
            }
            Msg::Check => self.check = None,
            Msg::Tick => self.extra.tick = None,
            Msg::Loop | Msg::Station(_) | Msg::ModelSignals(_) => {}
        }
        if let Some(ending) = self.ending() {
            if self.ended_at.is_none() {
                self.ended_at = Some(t);
                tracing::info!(?ending, "the dictation ended");
            }
        }
        let had_delivery = self.life.delivered.is_some();
        self.sync_lifecycle(t);
        if !had_delivery {
            // The receipt's app icon (EG-24) joins the view when the receipt starts.
            if let Some(wait) = self.receipt_wake(t) {
                cx.after(wait + 0.01, Msg::Loop);
            }
        }
        if let Some(ending) = self.ending() {
            // The idle check runs again when the linger ends; a delivery seen later
            // lengthens it to the end of the hand-off.
            let rearm = had_delivery != self.life.delivered.is_some();
            if ending.lingers() && (!self.linger_armed || rearm) {
                self.linger_armed = true;
                let left = self.linger_secs() - self.ended_for(t).unwrap_or(0.0);
                cx.after(left.max(0.0) + 0.01, Msg::Check);
            }
        }
        self.sync_showcase(t);
        self.sync_amber(t);
        self.sync_radial(t);
        self.sync_tron(t);
        self.sync_phosphor(t);
        self.kit_sync(t);
        self.sync_sounding(t);
        self.sync_tally(t);
        self.sync_scope(t);
        self.sync_captions(t);
        self.sync_karaoke(t);
        self.sync_island(t);
        self.sync_jarvis(t);
        self.sync_glass(t);
        self.sync_cyber(t);
        self.sync_synthwave(t);
        self.sync_endings(t);
        self.sync_hearing();
        self.sync_set_type(t);
        self.arm_tick(cx);
        self.arm_set_type_tick(cx);
        self.arm_scope_tick(cx);
        self.arm_captions_tick(cx);
        self.arm_karaoke_tick(cx);
        self.arm_island_tick(cx);
        self.arm_hologram_tick(cx);
        self.arm_jarvis_wake(cx);
        self.arm_glass_wake(cx);
        self.arm_cyber_tick(cx);
        self.arm_dotmatrix_tick(cx);
        self.pace = self.room_tone_pace(t);
        if !tick && self.idle(t) {
            tracing::info!(
                recording = self.files.recording,
                status = self.files.finishing,
                suppressed = self.files.suppressed,
                since_audio = self.last_audio.map(|a| t - a),
                "closing: the daemon is idle"
            );
            self.closing = true;
            if !self.in_wall {
                cx.exit();
            }
        }
    }

    fn surfaces(&self, _: &vo::Tokens) -> Vec<vo::SurfaceView<Msg>> {
        if self.look.theme == OsdTheme::Captions {
            let mut surfaces = self.captions_surfaces();
            for s in &mut surfaces {
                let view = std::mem::replace(&mut s.view, Element::column("set-type"));
                s.view = view.map_live_texts(&mut |e| self.set_type(e));
            }
            return surfaces;
        }
        Vec::new()
    }

    fn frame_pace(&self) -> Option<f64> {
        self.pace
    }

    fn key(&mut self, ev: &vo::scene::input::KeyEvent, cx: &mut Cx<Msg>) -> bool {
        let text_box = edit_site::site(self.look.theme).text_box_in(self);
        let Some(link) = self.edit.as_mut() else {
            return false;
        };
        // Up, Down, Home and End move by the lines the look draws.
        let lines = match link.buffer() {
            Some(b) if self.edit_view.is_some() => Some(self.edit_layer.shaped(
                b.text(),
                &self.measure,
                text_box.style,
                text_box.width,
            ) as &dyn vo_core::edit::Lines),
            _ => None,
        };
        let before: HashSet<vo_core::words::WordId> =
            self.model.words().iter().map(|w| w.id).collect();
        let used = self.focus.key(ev, link, lines);
        self.focus.follow(link, cx, ev.t);
        if used {
            self.sync_quiet();
            if self.edit_text(ev.t) {
                // A word the key made or changed has a new id.
                let new = self
                    .model
                    .words()
                    .iter()
                    .map(|w| w.id)
                    .filter(|id| !before.contains(id));
                self.edit_marks.extend(new);
                self.mark_edits();
            }
        }
        used
    }

    fn keyboard_focus(&mut self, gained: bool, requested: bool, cx: &mut Cx<Msg>) {
        if let Some(link) = self.edit.as_mut() {
            let t = cx.t();
            self.focus.changed(gained, requested, link, t);
            self.focus.follow(link, cx, t);
            self.sync_quiet();
            self.edit_text(t);
        }
    }

    /// A look picked with the look chord has its own size and place. A
    /// scripted run renders at the look's own size, so its frames follow a
    /// look switch as the live surface does.
    fn main_surface(&self) -> Option<(Size, Placement)> {
        match &self.osd {
            Some(osd) => {
                let spec = live_spec(osd, &self.look);
                Some((spec.size, spec.placement))
            }
            None if self.scripted => Some((self.look.surface(), Placement::Centered)),
            None => None,
        }
    }

    fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
        let look = self
            .look_view(tokens)
            .map_live_texts(&mut |e| self.set_type(e));
        // Each look hangs under a node named for it, so a look switch drops
        // the old look whole at once: no exit of it, its status row
        // included, draws over the new look.
        let look = match &self.edit_view {
            Some(v) => edit_site::site(self.look.theme).place(&self.look, look, v),
            // A pending tap or a detached edit: still, before and after.
            None if !self.look.quiet.is_empty() => {
                look.map_live_texts(&mut |e| e.still_motion(true))
            }
            None => look,
        };
        // Edited words stay underlined until delivery.
        let look = match &self.edit_view {
            None if !self.edit_marks.is_empty() => edit_site::place_marks(
                edit_site::site(self.look.theme),
                &self.look,
                look,
                &self.edit_marks,
            ),
            _ => look,
        };
        let look = correction_note::dim(look, self.correction_view());
        let look = Element::column(format!("look-{:?}", self.look.theme)).child(look);
        // The shift and the taller panel come in the same frame, on a
        // surface that keeps its size, so the reading line never moves.
        let root = match self.look.reading_shift(self.lines) {
            Some(shift) => Element::column("reading")
                .pad_edges(shift, 0.0, 0.0, 0.0)
                .child(look),
            None => look,
        };
        let frame = self.look.live_frame();
        let root = if self.live_surface && (self.look.badge_h() > 0.0 || frame.left > 0.0) {
            let strip = Element::column("badged").pad_edges(frame.top, 0.0, 0.0, frame.left);
            match self.look_badge(frame.badge_top, frame.badge_left) {
                Some(badge) => strip.child(badge),
                None => strip,
            }
            .child(root)
        } else {
            root
        };
        root.signals(self.signals.clone())
    }
}

/// Height of the look badge's strip above the panel.
const BADGE_H: f32 = 30.0;

impl Popup {
    /// "3/27 Type": the look's place in the look chord's list and its
    /// name, in bright text on a dark pill at the strip's left, so a chord press shows
    /// where it landed. With a keyboard edit, the edit's state after it,
    /// and while editing the keys' hint. None when it has nothing to say.
    fn look_badge(&self, top: f32, left: f32) -> Option<Element<Msg>> {
        let theme = self.look.theme;
        let name = format!("{theme:?}");
        let look =
            self.look.badge.then(
                || match OsdTheme::VO_LOOKS.iter().position(|t| *t == theme) {
                    Some(i) => format!("{}/{}  {name}", i + 1, OsdTheme::VO_LOOKS.len()),
                    None => name,
                },
            );
        let (edit, dim) = self.edit_badge().unzip();
        let text = match (look, edit) {
            (Some(l), Some(e)) => format!("{l} \u{b7} {e}"),
            (Some(l), None) => l,
            (None, Some(e)) => e.to_string(),
            (None, None) => return None,
        };
        let ink = |a: f32| Color::rgba(1.0, 1.0, 1.0, a);
        let mut row = Element::row(format!("look-badge-{theme:?}"))
            .align_items(AlignItems::CENTER)
            .pad_edges(3.0, 10.0, 3.0, 10.0)
            .radius(5.0)
            .fill(Color::rgba(0.0, 0.0, 0.0, 0.86))
            .layout(|s| {
                s.position = vo::scene::taffy::Position::Absolute;
                s.inset.left = vo::scene::taffy::LengthPercentageAuto::length(left);
                s.inset.top = vo::scene::taffy::LengthPercentageAuto::length(top + 1.0);
            })
            .child(
                Element::label("look-badge-text", text)
                    .text_style(TextStyle::new(SANS, 15.0, 20.0).tracking(0.3))
                    .color(ink(if dim == Some(true) { 0.5 } else { 1.0 })),
            );
        if let Some(a) = self.edit_hint() {
            row = row.child(
                Element::label("edit-hint", EDIT_HINT)
                    .text_style(TextStyle::new(SANS, 15.0, 20.0).tracking(0.3))
                    .color(ink(a))
                    .pad_edges(0.0, 0.0, 0.0, 10.0),
            );
        }
        Some(row)
    }

    /// The edit's state for the badge, and whether it is dimmed (plan
    /// section 2.1): a dimmed "EDIT" while a tap is pending, so keys typed
    /// then are known to go to the app (K7).
    fn edit_badge(&self) -> Option<(&'static str, bool)> {
        use edit_link::Phase;
        let link = self.edit.as_ref()?;
        Some(match link.phase()? {
            Phase::Listening => return None,
            Phase::Pending => ("EDIT", true),
            Phase::Settling => ("EDIT \u{b7} settling", false),
            Phase::Editing => ("EDIT \u{b7} paused", false),
            Phase::Detached => ("paused \u{b7} Shift to edit", false),
            Phase::Releasing if link.sending() => ("sending", false),
            Phase::Releasing => ("listening", false),
        })
    }

    /// The hint's ink while editing, for the first [`EDIT_HINT_USES`]
    /// edits this popup has shown, fading with each; None after them.
    fn edit_hint(&self) -> Option<f32> {
        let n = self.edit_entries;
        let editing = self.edit.as_ref().and_then(|l| l.phase()) == Some(edit_link::Phase::Editing);
        (editing && (1..=EDIT_HINT_USES).contains(&n))
            .then(|| 0.7 * (EDIT_HINT_USES + 1 - n) as f32 / EDIT_HINT_USES as f32)
    }
}

/// The keys' hint beside the edit badge.
const EDIT_HINT: &str = "Shift: listen \u{b7} Enter: send";
/// Edits that show the hint, each fainter than the last.
const EDIT_HINT_USES: u32 = 3;

impl Popup {
    /// The look chord was pressed: show the next (or previous) look at
    /// once, keep it for later popups and name it in the status row. The
    /// words, levels and timers stay as they are; `main_surface` gives vo
    /// the new look's size and place.
    fn step_look(&mut self, forward: bool, t: f64, cx: &mut Cx<Msg>) {
        if self.osd.is_none() && !self.scripted {
            return;
        }
        let theme = look_step::step(&OsdTheme::VO_LOOKS, self.look.theme, forward);
        tracing::info!(from = ?self.look.theme, to = ?theme, "look chord");
        self.look.theme = theme;
        // Only the live popup keeps the pick for later popups.
        if self.osd.is_some() {
            look_step::save(theme);
        }
        // The words wrap afresh to the new look's width and line count.
        self.lines = 1;
        self.cut = 0;
        self.grew = None;
        let preview = self.files.preview.clone();
        self.text(&preview);
        self.picked_at = Some(t);
        cx.after(LOOK_NAME_SECS + 0.01, Msg::Loop);
    }

    fn look_view(&self, _: &vo::Tokens) -> Element<Msg> {
        if self.look.theme == OsdTheme::Amber {
            return self.amber_view();
        }
        if self.look.theme == OsdTheme::Radial {
            return self.radial_view();
        }
        if self.look.theme == OsdTheme::Tron {
            return self.tron_view();
        }
        if self.look.theme == OsdTheme::Phosphor {
            return self.phosphor_view();
        }
        if self.look.theme == OsdTheme::Island {
            return self.island_view();
        }
        if self.look.theme == OsdTheme::Karaoke {
            return self.karaoke_view();
        }
        if self.look.theme == OsdTheme::Captions {
            return self.captions_view();
        }
        if self.look.theme == OsdTheme::Scope {
            return self.scope_view();
        }
        if self.look.theme == OsdTheme::Showcase {
            return self.showcase_view();
        }
        if self.look.theme == OsdTheme::Cyberpunk {
            return self.cyber_view();
        }
        if self.look.theme == OsdTheme::Type {
            return self.type_view();
        }
        if self.look.theme == OsdTheme::Mission {
            return self.mission_view();
        }
        if self.look.theme == OsdTheme::Palette {
            return self.palette_view();
        }
        if self.look.theme == OsdTheme::Dotmatrix {
            return self.dotmatrix_view();
        }
        if self.look.theme == OsdTheme::Swissfi {
            return self.swissfi_view();
        }
        if self.look.theme == OsdTheme::Jarvis {
            return self.jarvis_view();
        }
        if self.look.theme == OsdTheme::Particles {
            return self.particles_view();
        }
        if self.look.theme == OsdTheme::Synthwave {
            return self.synthwave_view();
        }
        if self.look.theme == OsdTheme::Mecha {
            return self.mecha_view();
        }
        match self.look.theme {
            OsdTheme::Orb => self.orb_view(),
            OsdTheme::Chrome => self.chrome_view(),
            OsdTheme::Glass => self.glass_view(),
            OsdTheme::Biolume => self.biolume_view(),
            OsdTheme::Aurora => self.aurora_view(),
            OsdTheme::Hologram => self.hologram_view(),
            OsdTheme::River => self.river_view(),
            _ => self.swiss_view(),
        }
    }

    /// The swiss panel: the swiss looks, and a new look's slot until its
    /// look bead lands (RD-03).
    pub(crate) fn swiss_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.panel())
    }
}

/// Swiss: nothing animates unless a view asks for it, as in GTK. Showcase
/// and type: everything springs.
/// The live surface's margin policy: `Auto` for every look except the ones
/// that ask for a minimum so entry causes no resize.
pub(crate) fn surface_margin(look: &Look) -> vo::SurfaceMargin {
    match look.theme {
        OsdTheme::Showcase => Popup::showcase_margin(look.live_frame().top),
        _ => vo::SurfaceMargin::default(),
    }
}

fn default_motion(look: &Look) -> Motion {
    if matches!(
        look.theme,
        OsdTheme::Showcase
            | OsdTheme::Type
            | OsdTheme::Amber
            | OsdTheme::Radial
            | OsdTheme::Tron
            | OsdTheme::Phosphor
            | OsdTheme::Mission
            | OsdTheme::Palette
            | OsdTheme::Dotmatrix
            | OsdTheme::Swissfi
            | OsdTheme::Scope
            | OsdTheme::Captions
            | OsdTheme::Karaoke
            | OsdTheme::Island
            | OsdTheme::Jarvis
            | OsdTheme::Particles
            | OsdTheme::Synthwave
            | OsdTheme::Mecha
            | OsdTheme::Cyberpunk
            | OsdTheme::River
            | OsdTheme::Hologram
            | OsdTheme::Glass
            | OsdTheme::Chrome
            | OsdTheme::Orb
    ) {
        Motion::Spring(Spring::SOFT)
    } else {
        Motion::Snap
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

/// A press of the look chord that this popup has not seen yet. The first
/// files a popup gets (`seen` false) only say where the chord stands: a press
/// from before the popup opened does nothing.
fn new_step(
    seen: bool,
    last: Option<look_step::Step>,
    next: Option<look_step::Step>,
) -> Option<look_step::Step> {
    next.filter(|_| seen && next != last)
}

/// The live popup's surface for `look`.
fn live_spec(cfg: &OsdConfig, look: &Look) -> OverlaySpec {
    OverlaySpec {
        size: look.live_surface(),
        placement: placement(cfg, look),
        input: InputMode::None,
        app_id: "voxtype-osd".into(),
        ..OverlaySpec::default()
    }
}

fn placement(cfg: &OsdConfig, look: &Look) -> Placement {
    use vo::Edges;
    let m = cfg.margin_px as i32;
    let corner = |edges: Edges| Placement::Anchored {
        edges,
        margin: [m, m, m, m],
    };
    match cfg.position {
        OsdPosition::BottomCenter | OsdPosition::TopCenter if cfg.anchor == OsdAnchor::Center => {
            Placement::LineAt {
                fraction: cfg.reading_line.clamp(0.0, 1.0),
                y: look.live_frame().line,
            }
        }
        OsdPosition::BottomCenter | OsdPosition::TopCenter => match look.reading_line {
            Some(fraction) => Placement::LineAt {
                fraction,
                y: look.badge_h()
                    + look
                        .newest_line_centre(look.max_lines())
                        .unwrap_or(look.surface().h / 2.0),
            },
            None => Placement::FromTop {
                fraction: cfg.top_margin.clamp(0.0, 1.0),
            },
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
    audio: Vec<(Vec<AudioFrame>, Vec<Voice>)>,
    files: Files,
    /// The live model's latest step, for the next popup.
    model_signals: Option<voxtype::osd::live::ModelSignals>,
    /// The compositor's socket was created since the last warm-up.
    display_new: bool,
}

/// Most audio batches held while no popup is open: about a second.
const HELD_AUDIO: usize = 64;

impl Sink {
    /// Sends one capture batch to the open popup, or keeps it for the next one.
    fn audio(&mut self, frames: Vec<AudioFrame>, voice: Vec<Voice>) {
        if let Some(p) = &self.proxy {
            if p.send(Msg::Audio(frames.clone(), voice.clone())) {
                return;
            }
            self.proxy = None;
        }
        self.audio.push((frames, voice));
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

    fn model_signals(&mut self, s: voxtype::osd::live::ModelSignals) {
        if let Some(p) = &self.proxy {
            if !p.send(Msg::ModelSignals(Box::new(s.clone()))) {
                self.proxy = None;
            }
        }
        self.model_signals = Some(s);
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
/// batch, with the voice features when the daemon sends version 2 frames.
/// Reconnects when the daemon goes away.
fn read_audio(shared: Shared, socket: PathBuf, reconnect: Duration) {
    realtime::register_thread();
    loop {
        if let Ok(mut reader) = UnixStream::connect(&socket).and_then(AudioReader::new) {
            tracing::info!(
                version = reader.version(),
                "connected to {}",
                socket.display()
            );
            while let Ok(rich) = reader.read() {
                let voice: Vec<Voice> = rich
                    .iter()
                    .filter_map(|r| {
                        r.features.map(|features| Voice {
                            seq: r.frame.seq,
                            features,
                            peak_dbfs: r.frame.peak_dbfs,
                            clock: r.sample_clock,
                        })
                    })
                    .collect();
                let frames = rich.into_iter().map(|r| r.frame).collect();
                let (lock, cv) = &*shared;
                let mut sink = lock.lock().unwrap();
                sink.audio(frames, voice);
                drop(sink);
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
    let state = std::fs::read_to_string(dir.join("state")).unwrap_or_default();
    // A state file this old is left over from a crash.
    let state_fresh = std::fs::metadata(dir.join("state"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < STATUS_STALE);
    Files {
        recording: matches!(state.trim(), "recording" | "streaming"),
        transcribing: state.trim() == "transcribing" && state_fresh,
        outcome: voxtype::osd::outcome::read(),
        delivery: voxtype::osd::outcome::read_delivery(),
        not_pasted: voxtype::osd::outcome::read_not_pasted(),
        finishing,
        suppressed: dir.join("osd_suppressed").exists(),
        preview: std::fs::read_to_string(VoxtypeConfig::live_preview_path())
            .unwrap_or_default()
            .trim()
            .to_string(),
        fraction: fraction.trim().parse::<f64>().ok(),
        status: text.trim().to_string(),
        live: voxtype::osd::live::read(),
        look_step: look_step::read(),
    }
}

/// Reads the runtime files and passes on each change: at once when inotify
/// reports one, and every `POLL_INTERVAL` anyway, which also ages out a stale
/// status.
fn poll_files(shared: Shared) {
    realtime::register_thread();
    let mut watch = voxtype::osd::watch::RuntimeWatch::new(VoxtypeConfig::runtime_dir());
    let mut last = None;
    let mut last_signals = None;
    loop {
        let files = read_files();
        if last.as_ref() != Some(&files) {
            last = Some(files.clone());
            let (lock, cv) = &*shared;
            lock.lock().unwrap().files(files);
            cv.notify_one();
        }
        let signals = voxtype::osd::live::read_signals();
        if signals != last_signals {
            last_signals = signals.clone();
            let (lock, _) = &*shared;
            let mut sink = lock.lock().unwrap();
            match signals {
                Some(s) => sink.model_signals(s),
                // The dictation ended: nothing to hand the next popup.
                None => sink.model_signals = None,
            }
        }
        watch.wait(POLL_INTERVAL);
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
    #[cfg(feature = "osd-vo-preview")]
    if vo::preview::wanted() {
        return replay::preview_main();
    }
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
    cfg.word_fade |= args.word_fade;
    cfg.look_badge |= args.look_badge;
    if let Some(r) = args.realtime {
        cfg.realtime = r;
    }
    if let Some(d) = args.doubt_below {
        cfg.doubt_below = d;
    }
    if let Some(a) = args.anchor.as_deref().and_then(OsdAnchor::from_name) {
        cfg.anchor = a;
    }
    if let Some(r) = args.reading_line {
        cfg.reading_line = r;
    }
    if let Some(t) = args.theme.as_deref().and_then(OsdTheme::parse_str) {
        cfg.theme = t;
    }
    let theme = plasma::load();
    let fonts = load_fonts(&theme)?;
    let mut look = Look::new(&cfg, &theme, stop_hint(&hotkey), &fonts);
    look.set_motion(&OsdMotion::layered(&args.motion, &cfg.motion));

    if args.moment.as_deref() == Some("list") {
        println!("{}", moments::Moment::names().replace(", ", "\n"));
        return Ok(());
    }
    if args.station {
        return station::run(&args, &cfg, &theme, stop_hint(&hotkey), fonts);
    }
    if args.wall {
        return run_wall(&args, &cfg, &theme, stop_hint(&hotkey), fonts);
    }
    if args.envelope {
        return envelope::run(look, fonts, args.fps, args.cancel, args.moment.as_deref());
    }
    if args.entry_latency {
        return entry_latency::run(look, fonts, args.fps, args.moment.as_deref());
    }
    if args.forming_latency {
        return entry_latency::forming(look, fonts, args.fps, args.moment.as_deref());
    }
    if args.exit_check {
        return exit_check(look, fonts, args.fps, args.cancel, args.moment.as_deref());
    }
    if let (Some(dir), Some(m)) = (&args.offline, &args.moment) {
        return moments::offline(look, fonts, dir, args.fps, m, args.gpu_times);
    }
    if let Some(m) = args.live_moment.as_deref() {
        return moments::live(look, fonts, &cfg, m);
    }
    if let Some(dir) = &args.offline {
        return offline(look, fonts, dir, args.fps, args.cancel, args.gpu_times);
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

    // The look picked with the look chord wins over the config and --theme.
    if let Some(theme) = look_step::saved() {
        look.theme = theme;
    }
    // The display connection, GPU device and renderer stay warm between
    // dictations, so every popup after the first draws on the GPU at once.
    let mut resident = vo::Resident::new();
    let mut retry = display::Retry::default();
    if !args.no_prewarm {
        prewarm(&mut resident, &look, &fonts, &mut retry);
        // A compositor that starts after us, or restarts: get ready again.
        let shared = shared.clone();
        display::watch(move || {
            let (lock, cv) = &*shared;
            lock.lock().unwrap().display_new = true;
            cv.notify_all();
        });
    }
    loop {
        // Wait for a dictation, then hand everything held so far to a new
        // popup and route later messages straight to it. While hidden, get
        // ready again when the display comes or a failed warm-up is due.
        let inbox = {
            let (lock, cv) = &*shared;
            let mut sink = lock.lock().unwrap();
            while !sink.wants_popup() && !sink.display_new && !retry.due() {
                sink = match retry.wait() {
                    Some(t) => cv.wait_timeout(sink, t).unwrap().0,
                    None => cv.wait(sink).unwrap(),
                };
            }
            if !sink.wants_popup() {
                let new_display = std::mem::take(&mut sink.display_new);
                drop(sink);
                if new_display {
                    tracing::info!("the display started; getting the popup ready");
                    resident.release();
                }
                prewarm(&mut resident, &look, &fonts, &mut retry);
                continue;
            }
            // The show connects to the compositor that is there now.
            sink.display_new = false;
            let (proxy, inbox) = vo::channel();
            // Audio first: frames that come before the daemon's state file
            // says "recording" must not let the popup close at once.
            for (frames, voice) in sink.audio.drain(..) {
                proxy.send(Msg::Audio(frames, voice));
            }
            proxy.send(Msg::Files(sink.files.clone()));
            if let Some(s) = sink.model_signals.take() {
                proxy.send(Msg::ModelSignals(Box::new(s)));
            }
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
            default_motion: default_motion(&look),
            renderer_setup: Some(Arc::new(vo_3d::install)),
            margin: surface_margin(&look),
            ..Options::default()
        };
        let mut popup = Popup::new(look.clone(), &fonts);
        popup.recorder = args.record.as_deref().and_then(replay::recorder);
        popup.osd = Some(cfg.clone());
        popup.live_surface = true;
        if cfg.keyboard_edit {
            popup.edit = Some(edit_link::EditLink::new(edit_link::Paths::default()));
        }
        let boost = realtime::raise(cfg.realtime);
        let result = resident.run(popup, live_spec(&cfg, &look), options);
        drop(boost);
        // The popup may have picked a look with the look chord.
        if let Some(theme) = look_step::saved() {
            look.theme = theme;
        }
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
                if !args.no_prewarm {
                    retry.failed();
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Frames of a scripted dictation, four per second, for the start-up
/// warm-up: drawing them once builds the shaders and glyphs a real
/// dictation needs. The long-silence moment adds what only a long pause
/// draws, such as the gathered pause marks, so their pipelines are not
/// built in the middle of a dictation.
/// Connects and makes the GPU device and renderer in the background, so
/// the next popup draws on the GPU at once. A failure is retried later.
fn prewarm(
    resident: &mut vo::Resident,
    look: &Look,
    fonts: &FontRegistry,
    retry: &mut display::Retry,
) {
    let warm = Options::<Msg> {
        fonts: fonts.clone(),
        renderer_setup: Some(Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let (look, fonts) = (look.clone(), fonts.clone());
    match resident.warm_up(&warm, move || warm_frames(look, fonts)) {
        Ok(_) => retry.succeeded(),
        // Not fatal: tried again later, and a dictation connects anyway.
        Err(e) => {
            if retry.failed() {
                tracing::warn!("cannot get the popup ready yet: {e}");
            } else {
                tracing::debug!("cannot get the popup ready yet: {e}");
            }
        }
    }
}

fn warm_frames(look: Look, fonts: FontRegistry) -> vo::WarmFrames {
    // The scripted dictation ends and delivers like a real one; its log
    // lines would read as a real delivery.
    tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
        warm_frames_quietly(look, fonts)
    })
}

fn warm_frames_quietly(look: Look, fonts: FontRegistry) -> vo::WarmFrames {
    let size = look.surface();
    let options = || Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        ..Options::default()
    };
    let mut lists = Vec::new();
    vo::envelope::sample_scenario(
        Popup::new(look.clone(), &fonts),
        &scenario(&look, false),
        4.0,
        options(),
        &mut |_, _, list| lists.push(list.clone()),
    );
    vo::envelope::sample_scenario(
        Popup::scripted(look.clone(), &fonts),
        &moments::scenario(&look, moments::Moment::LongSilence),
        4.0,
        options(),
        &mut |_, _, list| lists.push(list.clone()),
    );
    vo::WarmFrames {
        lists,
        size: [size.w.ceil() as u32, size.h.ceil() as u32],
        scale: 1.0,
    }
}

/// A scripted dictation: model loading, words arriving, a revised word, a
/// pause, the stop, the final text and the close. With `cancel` the
/// recording is cancelled at the pause: no final text, the popup closes.
fn scenario(look: &Look, cancel: bool) -> vo::Scenario<Msg> {
    let mut inputs: Vec<(f64, Input<Msg>)> = Vec::new();
    let msg = |m: Msg| Input::Msg(m);
    // Words are timed for the whole sentence, so a cancel cuts it off.
    let (start, full_stop) = (0.2, 6.6);
    let (stop, done) = if cancel { (3.4, 3.4) } else { (full_stop, 8.6) };
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
        inputs.push((t, msg(Msg::Audio(frames, Vec::new()))));
        t += 0.02;
    }
    let mut pump = 0u32;
    let mut t = stop + 0.03;
    while t < done && !cancel {
        let silent = AudioFrame {
            seq: pump,
            min: 0.0,
            max: 0.0,
            peak_dbfs: -120.0,
        };
        inputs.push((t, msg(Msg::Audio(vec![silent], Vec::new()))));
        pump += 1;
        t += 0.033;
    }
    // Text and status, as the file poller would see them.
    // VOXTYPE_OSD_VO_SENTENCE replaces the sentence, to match a mock-up's text.
    let custom = std::env::var("VOXTYPE_OSD_VO_SENTENCE").ok();
    let sentence = custom.as_deref().unwrap_or("open the configuration file and set the popup theme to swiss so the dictation \
                    window shows the live words in the middle of the screen with the waveform above \
                    them then press the key again to stop and let the final pass fix the casing and \
                    the punctuation before the text is typed into the editor");
    let words: Vec<&str> = sentence.split(' ').collect();
    let speaking_time = (full_stop - start - 0.5) - (pause.1 - pause.0);
    let per_word = speaking_time / words.len() as f64;
    let mut files = Files {
        recording: true,
        ..Files::default()
    };
    // Word facts as preview.json gives them. The newest word is unsure when
    // it is one the engine often gets wrong, and "popup" stays unsure until
    // the final pass.
    let mut doc = voxtype::osd::live::LiveDoc::default();
    let mut facts = |text: &str| {
        let words: Vec<&str> = text.split_whitespace().collect();
        let timed: Vec<voxtype::osd::live::TimedWord> = words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let newest = i + 1 == words.len();
                let unsure =
                    *w == "popup" || (newest && matches!(*w, "swish" | "confi" | "dictation"));
                voxtype::osd::live::TimedWord {
                    text: w.to_string(),
                    start: i as f32 * 0.3,
                    end: i as f32 * 0.3 + 0.25,
                    conf: Some(if unsure { 0.4 } else { 0.93 }),
                }
            })
            .collect();
        doc.apply_preview(text, Some(&timed));
        Some(doc.clone())
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
        if t >= stop {
            break;
        }
        let mut text = words[..=i].join(" ");
        // A word heard wrong at first, fixed when the next one comes.
        if words[i] == "swiss" {
            text = format!("{} swish", words[..i].join(" "));
        }
        // A word still being spoken shows its start first.
        if words[i] == "configuration" {
            files.preview = format!("{} confi", words[..i].join(" "));
            files.live = facts(&files.preview);
            at(t, &files, &mut inputs);
            t += per_word * 0.5;
        }
        if i + 1 == words.len() {
            text = text.trim().to_string();
        }
        files.live = facts(&text);
        files.preview = text;
        at(t, &files, &mut inputs);
        t += per_word;
    }
    files.recording = false;
    if cancel {
        files.outcome = Some(Outcome::Cancelled);
        at(stop, &files, &mut inputs);
        inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
        return vo::Scenario {
            size: look.surface(),
            scale: 1.0,
            duration: stop + 1.0,
            inputs,
        };
    }
    // Stop: the final text is made, then pasted.
    files.transcribing = true;
    at(stop, &files, &mut inputs);
    files.finishing = true;
    for (k, f) in [0.1, 0.35, 0.6, 0.85, 1.0].iter().enumerate() {
        files.fraction = Some(*f);
        files.status = format!("Finishing: {} s of audio left", 4 - k.min(4));
        at(stop + 0.05 + k as f64 * 0.35, &files, &mut inputs);
    }
    let mut fin: String = sentence
        .replace(" so the", ", so the")
        .replace(" then press", ". Then press");
    let first = fin[..1].to_uppercase();
    fin.replace_range(0..1, &first);
    fin.push('.');
    files.preview = fin;
    at(done - 0.4, &files, &mut inputs);
    files.finishing = false;
    files.transcribing = false;
    files.fraction = None;
    files.status.clear();
    files.delivery = Some(demo_delivery(&files.preview));
    files.outcome = Some(Outcome::Done);
    at(done, &files, &mut inputs);
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    vo::Scenario {
        size: look.surface(),
        scale: 1.0,
        duration: done + 1.8,
        inputs,
    }
}

/// The demo's `result.json`. `VOXTYPE_OSD_VO_DELIVERY` picks how it went: `typed`
/// (the default), `typed-enter`, `paste`, `paste-enter` or `clipboard`.
fn demo_delivery(text: &str) -> voxtype::osd::outcome::Delivery {
    let how = std::env::var("VOXTYPE_OSD_VO_DELIVERY").unwrap_or_default();
    let method = match how.trim_end_matches("-enter") {
        "paste" => "paste",
        "clipboard" => "clipboard",
        _ => "wtype",
    };
    let chars = text.chars().count();
    voxtype::osd::outcome::Delivery {
        text: text.to_string(),
        chars,
        method: method.into(),
        enter: how.ends_with("-enter"),
        output_ms: if method == "wtype" { chars as u64 * 3 } else { 40 },
        latency_ms: Some(620),
        audio_s: Some(6.4),
    }
}

fn offline(
    look: Look,
    fonts: FontRegistry,
    dir: &Path,
    fps: f64,
    cancel: bool,
    gpu_times: bool,
) -> anyhow::Result<()> {
    let scenario = scenario(&look, cancel);
    let (times, sink) = gpu_time_sink(gpu_times);
    let options = Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        frame_timings: sink,
        renderer_setup: Some(Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let n = vo::render_offline_with(Popup::scripted(look, &fonts), &scenario, fps, dir, options)
        .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
    eprintln!("voxtype-osd-vo: wrote {n} frames to {}", dir.display());
    if gpu_times {
        print_gpu_times(&times.lock().unwrap());
    }
    eprintln!(
        "voxtype-osd-vo: heap peak {:.2} MB",
        mb(PEAK.load(Ordering::Relaxed))
    );
    Ok(())
}

/// `--exit-check`: runs the scripted dictation, or `moment`, on the CPU and
/// says whether the exit settled or vo's exit motion cap cut it. Exits 1 on
/// a failure.
fn exit_check(
    look: Look,
    fonts: FontRegistry,
    fps: f64,
    cancel: bool,
    moment: Option<&str>,
) -> anyhow::Result<()> {
    let scenario = match moment {
        Some(name) => {
            let m = moments::Moment::parse(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "no moment {name:?}; the moments are {}",
                    moments::Moment::names()
                )
            })?;
            moments::scenario(&look, m)
        }
        None => scenario(&look, cancel),
    };
    let name = format!("{:?}", look.theme).to_ascii_lowercase();
    let options = Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        ..Options::default()
    };
    let report = vo::exit_check(Popup::scripted(look, &fonts), &scenario, fps, options);
    match report.ending {
        vo::Ending::Settled { exit_at, at } => {
            println!(
                "exit-check: ok {name} settled {:.2} s after exit",
                at - exit_at
            );
            Ok(())
        }
        vo::Ending::Capped { .. } => {
            println!(
                "exit-check: FAIL {name} still moving {} s after exit",
                vo::Ending::CAP
            );
            std::process::exit(1);
        }
        vo::Ending::Unfinished { .. } => {
            println!(
                "exit-check: FAIL {name} never closed: the scripted dictation ended at {:.1} s and the popup did not ask to exit",
                scenario.duration
            );
            std::process::exit(1);
        }
    }
}

/// GPU milliseconds per frame, by surface: "main" or an extra surface's key.
type GpuTimes = Arc<Mutex<Vec<(String, f64)>>>;

/// The GPU times, and the sink that fills them when `on`.
fn gpu_time_sink(on: bool) -> (GpuTimes, Option<vo::FrameTimingsSink>) {
    let times: GpuTimes = Arc::default();
    let sink = times.clone();
    let f = on.then(|| {
        Arc::new(move |f: &vo::FrameTiming| {
            let key = f.surface.unwrap_or("main").to_string();
            sink.lock().unwrap().push((key, f.timings.total_ms));
        }) as vo::FrameTimingsSink
    });
    (times, f)
}

/// One line per surface: frames, and GPU milliseconds per frame as median,
/// 95th and 99th percentile and worst.
fn print_gpu_times(times: &[(String, f64)]) {
    if times.is_empty() {
        eprintln!("voxtype-osd-vo: no GPU times: this GPU has no timestamps inside encoders");
        return;
    }
    let mut keys: Vec<&str> = times.iter().map(|(k, _)| k.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let mut ms: Vec<f64> = times
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, m)| *m)
            .collect();
        ms.sort_by(f64::total_cmp);
        let at = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
        eprintln!(
            "voxtype-osd-vo: GPU {key}: {} frames, median {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
            ms.len(),
            at(0.5),
            at(0.95),
            at(0.99),
            ms[ms.len() - 1]
        );
    }
}

/// The looks the vo frontend draws itself.
const WALL_THEMES: &[OsdTheme] = &OsdTheme::VO_LOOKS;

/// `voxtype-osd-vo --wall`: see `osd_vo/wall.rs`.
fn run_wall(
    args: &Args,
    cfg: &OsdConfig,
    theme: &PlasmaTheme,
    stop_hint: String,
    fonts: FontRegistry,
) -> anyhow::Result<()> {
    let themes: Vec<OsdTheme> = if args.wall_themes.is_empty() {
        WALL_THEMES.to_vec()
    } else {
        args.wall_themes
            .iter()
            .map(|t| {
                OsdTheme::parse_str(t.trim()).ok_or_else(|| anyhow::anyhow!("unknown theme {t:?}"))
            })
            .collect::<anyhow::Result<_>>()?
    };
    let looks: Vec<Look> = themes
        .iter()
        .map(|&t| {
            let mut c = cfg.clone();
            c.theme = t;
            let mut look = Look::new(&c, theme, stop_hint.clone(), &fonts);
            look.set_motion(&OsdMotion::layered(&args.motion, &cfg.motion));
            look
        })
        .collect();
    let (w, h) = args
        .wall_size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
        .ok_or_else(|| anyhow::anyhow!("--wall-size wants WxH, for example 1800x1000"))?;
    let size = Size::new(w, h);
    let script = match args.wall_script.as_str() {
        "demo" => wall::Script::Demo,
        "live" => wall::Script::Live,
        s => match moments::Moment::parse(s) {
            Some(m) => wall::Script::Moment(m),
            None => anyhow::bail!(
                "--wall-script is demo, live or a moment ({}), not {s:?}",
                moments::Moment::names()
            ),
        },
    };
    let app = wall::Wall::new(looks, &fonts, args.wall_cols, size, script);
    let options = |inbox| Options {
        fonts: fonts.clone(),
        inbox,
        default_motion: Motion::Spring(Spring::SOFT),
        renderer_setup: Some(Arc::new(vo_3d::install)),
        ..Options::default()
    };
    if let Some(dir) = &args.offline {
        let scenario = vo::Scenario {
            size,
            scale: 1.0,
            duration: app.demo_len(),
            inputs: Vec::new(),
        };
        let n = vo::render_offline_with(app, &scenario, args.fps, dir, options(None))
            .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
        eprintln!("voxtype-osd-vo: wrote {n} wall frames to {}", dir.display());
        eprintln!(
            "voxtype-osd-vo: heap peak {:.2} MB",
            mb(PEAK.load(Ordering::Relaxed))
        );
        return Ok(());
    }
    let inbox = if script == wall::Script::Live {
        let shared: Shared = Arc::new((Mutex::new(Sink::default()), Condvar::new()));
        let socket = resolve_socket_path(args.socket.clone());
        let reconnect = Duration::from_secs_f32(args.reconnect_secs.max(0.05));
        let (proxy, inbox) = vo::channel();
        shared.0.lock().unwrap().proxy = Some(proxy);
        {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("voxtype-osd-audio".into())
                .spawn(move || read_audio(shared, socket, reconnect))?;
        }
        std::thread::Builder::new()
            .name("voxtype-osd-files".into())
            .spawn(move || poll_files(shared))?;
        Some(inbox)
    } else {
        None
    };
    let spec = OverlaySpec {
        size,
        placement: Placement::Fullscreen,
        input: InputMode::None,
        app_id: "voxtype-osd-wall".into(),
        ..OverlaySpec::default()
    };
    vo::run_overlay_with(app, spec, options(inbox)).map_err(|e| anyhow::anyhow!("wall: {e}"))
}

/// The words from `start` on. With `inline`, the first starts with "…" when
/// earlier words are cut.
fn tail(words: &[Word], start: usize, inline: bool) -> Vec<Word> {
    let mut shown = words[start..].to_vec();
    if inline && start > 0 {
        if let Some(first) = shown.first_mut() {
            first.text = format!("…{}", first.text);
        }
    }
    shown
}

/// The tail of `words` that fits in `max` lines, and where it starts. From
/// `start`, the whole first line of the layout is dropped until the rest fits,
/// so the text moves only when a line fills, and then by one line. `inline`
/// puts the "…" on the new first word, as [`tail`] does.
fn cut_lines(
    words: &[Word],
    start: usize,
    max: u32,
    inline: bool,
    lines_of: impl Fn(&[Word]) -> u32,
) -> (usize, Vec<Word>) {
    let last = words.len().saturating_sub(1);
    let mut start = start.min(last).max(words.len().saturating_sub(120));
    let mut shown = tail(words, start, inline);
    while start < last && lines_of(&shown) > max {
        let mut first_line = 1;
        while first_line < shown.len() && lines_of(&shown[..=first_line]) == 1 {
            first_line += 1;
        }
        start = (start + first_line).min(last);
        shown = tail(words, start, inline);
    }
    (start, shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word is unsure below `[osd] doubt_below`: at the measured default
    /// 0.45 a 0.5 word is not, at the old 0.6 it is.
    #[test]
    fn unsure_words_follow_doubt_below() {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let unsure = |doubt_below: f32| {
            let cfg = OsdConfig {
                theme: OsdTheme::Type,
                doubt_below,
                ..OsdConfig::default()
            };
            let look = Look::new(&cfg, &plasma, String::new(), &fonts);
            let mut p = Popup::new(look, &fonts);
            let text = "one two three";
            p.files.preview = text.into();
            p.text(text);
            let word = |w: &str, conf: f32| voxtype::osd::live::LiveWord {
                text: w.into(),
                start: None,
                end: None,
                conf: Some(conf),
                since: 0,
                changes: 0,
                detail: None,
            };
            p.files.live = Some(voxtype::osd::live::LiveDoc {
                text: text.into(),
                words: vec![word("one", 0.99), word("two", 0.5), word("three", 0.4)],
                ..Default::default()
            });
            p.word_info();
            p.note_doubtful(0.0);
            p.extra.doubtful.len()
        };
        assert_eq!(voxtype::osd::config::DOUBT_BELOW, 0.45);
        assert_eq!(unsure(voxtype::osd::config::DOUBT_BELOW), 1);
        assert_eq!(unsure(0.6), 2);
        assert_eq!(unsure(0.3), 0);
    }

    /// With the centre anchor the badge sits on the same spot for every
    /// look: the same distance above and left of the reading line's middle,
    /// on whole pixels, above every look's panel. Each panel's middle is on
    /// the reading line.
    #[test]
    fn the_badge_keeps_its_spot_in_every_look() {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let cfg = OsdConfig {
            anchor: OsdAnchor::Center,
            look_badge: true,
            width_px: 600,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        let spot = |look: &Look| {
            let f = look.live_frame();
            (f.line - f.badge_top, f.size.w / 2.0 - f.badge_left)
        };
        let first = spot(&look);
        assert_eq!(first.0.fract(), 0.0, "{first:?}");
        assert_eq!(OsdTheme::VO_LOOKS.len(), 27);
        for &theme in OsdTheme::VO_LOOKS.iter() {
            look.theme = theme;
            let f = look.live_frame();
            assert_eq!(spot(&look), first, "{theme:?}");
            assert_eq!(f.size.w % 2.0, 0.0, "{theme:?}: even width");
            assert_eq!(f.line, f.top + look.panel_middle(), "{theme:?}");
            let panel_top = f.top + look.panel_top();
            assert!(
                panel_top >= f.badge_top + BADGE_H,
                "{theme:?}: badge over the panel"
            );
            let fraction = cfg.reading_line;
            let y = f.line;
            assert_eq!(placement(&cfg, &look), Placement::LineAt { fraction, y });
        }
    }

    /// The warm-up's scripted dictation logs nothing, so the log shows only
    /// real dictations ending and delivering.
    #[test]
    fn the_warm_up_logs_nothing() {
        use std::io::Write;
        #[derive(Clone, Default)]
        struct Buf(Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let cfg = OsdConfig::default();
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let look = Look::new(&cfg, &plasma, String::new(), &fonts);
        let buf = Buf::default();
        let sink = buf.clone();
        let logger = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || sink.clone())
            .finish();
        let frames = tracing::subscriber::with_default(logger, || {
            tracing::info!("the logger works");
            warm_frames(look, fonts)
        });
        assert!(!frames.lists.is_empty());
        let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("the logger works"), "{log}");
        assert!(!log.contains("delivered"), "{log}");
        assert!(!log.contains("dictation ended"), "{log}");
    }

    /// t0xg: offline, the frames kept the first look's size for the whole
    /// run, so after the look chord showcase was cut off at swiss's size.
    /// A scripted run now asks for the look's own surface, and the frames
    /// follow it as the live surface does.
    #[test]
    fn a_scripted_look_switch_renders_at_the_new_looks_size() {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        for width_px in [400, 600] {
            let look_of = |theme| {
                let cfg = OsdConfig {
                    theme,
                    width_px,
                    ..OsdConfig::default()
                };
                let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
                look.set_motion(&cfg.motion);
                look
            };
            let (swiss, showcase) = (look_of(OsdTheme::Swiss), look_of(OsdTheme::Showcase));
            assert_ne!(swiss.surface(), showcase.surface());
            let s = moments::scenario(&swiss, moments::Moment::LookSwitch);
            let options = Options {
                fonts: fonts.clone(),
                default_motion: default_motion(&swiss),
                ..Options::default()
            };
            let mut sizes: Vec<Size> = Vec::new();
            let popup = Popup::scripted(swiss.clone(), &fonts);
            vo::envelope::sample_scenario(popup, &s, 10.0, options, &mut |_, _, dl| {
                if sizes.last() != Some(&dl.size) {
                    sizes.push(dl.size);
                }
            });
            assert_eq!(
                sizes,
                [swiss.surface(), showcase.surface()],
                "{width_px} px"
            );
        }
    }

    #[test]
    fn only_presses_while_the_popup_shows_step_the_look() {
        let press = |stamp| {
            Some(look_step::Step {
                stamp,
                forward: true,
            })
        };
        // A press from before the popup opened, in its first files.
        assert_eq!(new_step(false, None, press(1)), None);
        // A new press while it shows.
        assert_eq!(new_step(true, press(1), press(2)), press(2));
        // Files that changed for another reason.
        assert_eq!(new_step(true, press(2), press(2)), None);
        assert_eq!(new_step(true, None, None), None);
    }

    /// RD-11: on room tone the popup asks for `ROOM_TONE_FPS`, so a room-tone
    /// audio frame does not draw before its slot (vo's `live::pace_tests`),
    /// while the waveform still scrolls and keeps its history. Speech, a
    /// change of words or the end of the recording draws at once again.
    #[test]
    fn room_tone_frames_wait_for_the_ambient_pace() {
        let cfg = OsdConfig::default();
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let look = Look::new(&cfg, &plasma, String::new(), &fonts);
        let mut p = Popup::new(look, &fonts);
        p.files.recording = true;
        let mut seq = 0;
        let mut frames = |amp: f32| {
            seq += 2;
            vec![
                AudioFrame { seq: seq - 1, min: -amp, max: amp, peak_dbfs: 0.0 },
                AudioFrame { seq, min: -amp, max: amp, peak_dbfs: 0.0 },
            ]
        };
        // Speech: every vblank.
        let mut t = 1.0;
        for _ in 0..10 {
            t += 0.02;
            p.audio(frames(0.3), t);
            assert_eq!(p.room_tone_pace(t), None);
        }
        // Room tone, as the long-silence moment plays it (-54 dBFS): the
        // full rate while the speech peak's release scrolls, then the pace.
        let speech_end = t;
        let mut paced_at = None;
        let (mut newest, mut levels) = (p.newest_at, p.levels.len());
        for _ in 0..50 {
            t += 0.02;
            p.audio(frames(0.002), t);
            if paced_at.is_none() && p.room_tone_pace(t).is_some() {
                paced_at = Some(t);
            }
            // The waveform keeps scrolling and keeps its history.
            assert!(p.newest_at > newest && p.levels.len() >= levels);
            (newest, levels) = (p.newest_at, p.levels.len());
        }
        let paced_at = paced_at.expect("room tone is paced");
        assert!(paced_at - speech_end >= ROOM_TONE_AFTER_LOUD_SECS - 1e-9);
        assert_eq!(p.room_tone_pace(t), Some(ROOM_TONE_FPS));
        // The clock still ticks under the pace.
        assert_eq!(p.clock, format!("00:0{}", t as u64));
        // A new word: the full rate while its motion plays.
        p.event_at = t;
        assert_eq!(p.room_tone_pace(t + 0.3), None);
        assert!(p.room_tone_pace(t + ROOM_TONE_AFTER_EVENT_SECS).is_some());
        // Speech again: at once.
        t += 0.02;
        p.audio(frames(0.3), t);
        assert_eq!(p.room_tone_pace(t), None);
        // The recording stopped: endings and the exit keep the full rate.
        p.files.recording = false;
        assert_eq!(p.room_tone_pace(t + 5.0), None);
    }

    /// A frame put on the signal bus is the frame vo reads off the wire.
    #[test]
    fn voice_frames_match_the_wire_reading() {
        use voxtype::audio::levels::RichFrame;
        let mut mel = [0u8; 24];
        mel[3] = 200;
        mel[23] = 17;
        let features = AudioFeatures {
            rms_dbfs: -21.5,
            f0_hz: 140.0,
            voiced: true,
            centroid_hz: 900.0,
            flatness: 0.2,
            mel,
            onset: 0.7,
        };
        let frame = AudioFrame {
            seq: 5,
            min: -0.1,
            max: 0.1,
            peak_dbfs: -12.0,
        };
        let rich = RichFrame {
            frame,
            features: Some(features),
            sample_clock: 24_000,
        };
        let voice = Voice {
            seq: 5,
            features,
            peak_dbfs: frame.peak_dbfs,
            clock: rich.sample_clock,
        };
        let wire = SignalFrame::from_wire_v3(&rich.to_bytes_v3()).unwrap();
        assert_eq!(voice.signal_frame(), Some(wire));
        assert_eq!(wire.clock, 1.5);
        // Without a sample clock the bus cannot place the frame.
        assert_eq!(Voice { clock: 0, ..voice }.signal_frame(), None);
    }

    /// Frames pushed as they arrive reach the bus the scene reads; frames
    /// without a clock are left out.
    #[test]
    fn voice_feeds_the_signal_bus() {
        let signals = SignalsCell::default();
        let voice = |n: u64| Voice {
            seq: n as u32,
            features: AudioFeatures {
                rms_dbfs: -20.0,
                ..AudioFeatures::default()
            },
            peak_dbfs: -10.0,
            clock: 160 * (n + 1),
        };
        let mut batch: Vec<Voice> = (0..10).map(voice).collect();
        batch.push(Voice {
            clock: 0,
            ..voice(10)
        });
        feed_signals(&signals, &batch, 2.0);
        let bus = signals.get();
        assert_eq!(bus.len(), 10, "one batch keeps all its frames");
        let at: Vec<f64> = bus.frames().map(|(at, _)| at).collect();
        for pair in at.windows(2) {
            assert!(
                (pair[1] - pair[0] - 0.01).abs() < 1e-9,
                "10 ms apart: {at:?}"
            );
        }
        // The next batch carries on after the first.
        let next: Vec<Voice> = (10..20).map(voice).collect();
        feed_signals(&signals, &next, 2.1);
        assert_eq!(signals.get().len(), 20);
    }

    /// Lines of `words` wrapped greedily at `width` characters.
    fn wrapped(words: &[Word], width: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for w in words {
            match lines.last_mut() {
                Some(l) if l.chars().count() + 1 + w.text.chars().count() <= width => {
                    l.push(' ');
                    l.push_str(&w.text);
                }
                _ => lines.push(w.text.clone()),
            }
        }
        lines
    }

    /// As words arrive, the text only moves when a line fills, and then the
    /// whole first line goes; the new first word starts with "…".
    #[test]
    fn overflow_drops_the_whole_first_line() {
        let text = "the quick brown fox jumps over a lazy dog while seven tired \
                    cats watch from under the old green garden bench at noon today";
        let all: Vec<&str> = text.split_whitespace().collect();
        let lines_of = |shown: &[Word]| wrapped(shown, 18).len() as u32;
        let mut model = WordModel::new();
        let (mut cut, mut before) = (0, Vec::new());
        let mut moves = 0;
        for n in 1..=all.len() {
            model.update(&all[..n].join(" "));
            let words = model.words();
            let (next, shown) = cut_lines(words, cut, 3, true, lines_of);
            assert!(lines_of(&shown) <= 3, "fits in three lines");
            if next != cut {
                moves += 1;
                // The words dropped are exactly the first line on screen before.
                let first = wrapped(&before, 18)[0].split(' ').count();
                assert_eq!(next - cut, first, "a whole line goes at once");
                assert!(shown[0].text.starts_with('…'));
                assert_eq!(shown[0].text, format!("…{}", words[next].text));
            }
            (cut, before) = (next, shown);
        }
        assert!(moves >= 2, "the sample scrolls more than once");
    }

    /// With the "…" in the margin, a cut moves the text by exactly one line:
    /// every other line keeps its words, and no word carries the "…".
    #[test]
    fn a_margin_ellipsis_cut_is_one_block_scroll() {
        let text = "the quick brown fox jumps over a lazy dog while seven tired \
                    cats watch from under the old green garden bench at noon today";
        let all: Vec<&str> = text.split_whitespace().collect();
        let lines_of = |shown: &[Word]| wrapped(shown, 18).len() as u32;
        let mut model = WordModel::new();
        let (mut cut, mut before) = (0, Vec::new());
        let mut moves = 0;
        for n in 1..=all.len() {
            model.update(&all[..n].join(" "));
            let words = model.words();
            let (next, shown) = cut_lines(words, cut, 3, false, lines_of);
            assert!(shown.iter().all(|w| !w.text.contains('…')));
            if next != cut {
                moves += 1;
                let old = wrapped(&before, 18);
                let new = wrapped(&shown, 18);
                // All but the line still filling stay as they were, one row up.
                assert_eq!(new[..old.len() - 2], old[1..old.len() - 1]);
                assert!(new[old.len() - 2].starts_with(&old[old.len() - 1]));
            }
            (cut, before) = (next, shown);
        }
        assert!(moves >= 2, "the sample scrolls more than once");
    }

    #[test]
    fn a_plain_symbolic_icon_gives_its_view_box_and_paths() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
            <path style="fill:currentColor" d="M 8,2 L 10,4 Z" class="ColorScheme-Text"/>
            <path id="b" d="M 4,7 l 0,1 Z"/></svg>"#;
        let (view, d) = svg_icon_path(svg).unwrap();
        assert_eq!((view.x, view.y, view.w, view.h), (0.0, 0.0, 16.0, 16.0));
        assert_eq!(d, "M 8,2 L 10,4 Z M 4,7 l 0,1 Z");
        assert!(vo::scene::path::kurbo::BezPath::from_svg(&d).is_ok());
    }

    #[test]
    fn an_icon_with_transforms_or_other_shapes_is_refused() {
        let moved = r#"<svg viewBox="0 0 16 16"><g transform="translate(1 1)"><path d="M0 0h4v4z"/></g></svg>"#;
        let circle =
            r#"<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="2"/><path d="M0 0h4v4z"/></svg>"#;
        assert!(svg_icon_path(moved).is_none());
        assert!(svg_icon_path(circle).is_none());
    }
}
