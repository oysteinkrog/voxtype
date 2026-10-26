//! `voxtype-osd-gtk4` — GTK4 + gtk4-layer-shell on-screen mic visualizer
//! for the Voxtype daemon.
//!
//! Renders a click-through, layer-shell-anchored window containing the
//! scrolling waveform plus a segmented peak meter. Audio frames arrive on
//! the daemon's audio Unix socket via [`voxtype::osd::ipc::run_ipc_loop`],
//! decoded into [`AudioFrame`]s by a tokio runtime on a worker thread, and
//! pushed into a shared [`FrameRing`] + [`PeakHold`]. The GTK frame clock
//! redraws the `DrawingArea` while visible, including between audio arrivals.
//!
//! When the IPC socket is silent for `idle_timeout_secs` (Idle proxy) the
//! window is hidden so the binary does no rendering work and consumes
//! effectively zero CPU. It reappears when frames resume.
//!
//! Run with `RUST_LOG=debug` for verbose logs.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cairo::{Context, RectangleInt, Region};
use clap::Parser;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, DrawingArea};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use voxtype::audio::levels::{AudioFrame, FRAME_HZ};
use voxtype::config::{ActivationMode, Config as VoxtypeConfig, HotkeyConfig};
use voxtype::osd::config::{OsdConfig, OsdPosition, OsdTheme};
use voxtype::osd::gtk_env;
use voxtype::osd::ipc::{resolve_socket_path, run_ipc_loop, FrameRing};
use voxtype::osd::plasma::{self, PlasmaTheme};
use voxtype::osd::theme::ThemeWatcher;
use voxtype::osd::visual::{
    peak_meter_fraction, project_envelope, Color, MeterZone, Palette, PeakHold,
};

/// Load the `[osd]` section from the voxtype config file, falling back to
/// `OsdConfig::default()` on any error (file missing, unreadable, parse
/// failure, or `[osd]` section absent).
///
/// We deliberately ignore parse errors instead of returning them: the OSD
/// is a side car, and a malformed config shouldn't prevent it from running
/// with sensible defaults — the user will see the daemon complain about
/// the same file separately.
fn load_osd_config_from_file(explicit: Option<&std::path::Path>) -> OsdConfig {
    let path = explicit
        .map(std::path::Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path);
    let Some(path) = path else {
        return OsdConfig::default();
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return OsdConfig::default(),
    };

    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        osd: Option<OsdConfig>,
    }

    match toml::from_str::<PartialConfig>(&content) {
        Ok(p) => p.osd.unwrap_or_default(),
        Err(_) => OsdConfig::default(),
    }
}

/// Load `[hotkey]` for the stop hint in the `swiss` theme. Same fallback
/// rules as [`load_osd_config_from_file`].
fn load_hotkey_config_from_file(explicit: Option<&std::path::Path>) -> HotkeyConfig {
    #[derive(serde::Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        hotkey: Option<HotkeyConfig>,
    }

    explicit
        .map(std::path::Path::to_path_buf)
        .or_else(VoxtypeConfig::default_path)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| toml::from_str::<PartialConfig>(&content).ok())
        .and_then(|p| p.hotkey)
        .unwrap_or_default()
}

/// Application id for the GTK4 frontend.
const APP_ID: &str = "io.voxtype.OsdGtk4";

/// Visibility polling period. Drawing follows the GTK frame clock.
const RENDER_TICK_MS: u32 = 16;

/// How long we wait without frames before treating the daemon as idle and
/// hiding the surface. Matches the BRIEF's "Idle: surface destroyed" rule.
const IDLE_TIMEOUT_SECS: f32 = 0.15;

/// True while the daemon has marked the in-flight recording as OSD-suppressed
/// (`voxtype record start --no-osd`).
///
/// Frames stop flowing for a suppressed recording too, so the idle timeout
/// alone would hide the surface. This is checked anyway so suppression is
/// immediate rather than waiting out IDLE_TIMEOUT_SECS, and so the behaviour
/// matches the Quickshell frontend exactly.
fn osd_suppressed() -> bool {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    std::path::Path::new(&base)
        .join("voxtype/osd_suppressed")
        .exists()
}

/// Number of segments in the vertical peak meter.
const METER_SEGMENTS: usize = 10;

/// dBFS floor for the peak meter (maps to "empty bar").
const METER_FLOOR_DBFS: f32 = -60.0;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "voxtype-osd-gtk4",
    version,
    about = "Voxtype on-screen mic visualizer (GTK4 + gtk4-layer-shell)"
)]
struct Args {
    /// Path to the voxtype config file. Defaults to
    /// `~/.config/voxtype/config.toml`. Only the `[osd]` section is read.
    #[arg(long, env = "VOXTYPE_CONFIG")]
    config: Option<PathBuf>,

    /// Path to the audio-frame Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/voxtype/audio.sock`.
    #[arg(long, env = "VOXTYPE_OSD_SOCKET")]
    socket: Option<PathBuf>,

    /// Seconds to wait between reconnect attempts when the daemon is down.
    #[arg(long, default_value = "1.0", env = "VOXTYPE_OSD_RECONNECT_SECS")]
    reconnect_secs: f32,

    /// Print one debug line per N frames received (0 = quiet).
    #[arg(long, default_value = "0", env = "VOXTYPE_OSD_LOG_EVERY")]
    log_every: u32,

    /// Held-peak decay rate in dB/sec.
    #[arg(long, default_value = "6.0", env = "VOXTYPE_OSD_PEAK_DECAY")]
    peak_decay_db_per_sec: f32,

    /// Surface width in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_WIDTH")]
    width_px: Option<u32>,

    /// Surface height in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_HEIGHT")]
    height_px: Option<u32>,

    /// Margin from the screen edge in physical pixels.
    #[arg(long, env = "VOXTYPE_OSD_MARGIN")]
    margin_px: Option<u32>,

    /// Visual gain applied to audio samples before drawing the waveform.
    /// Higher = waveform fills more of the vertical for quiet inputs.
    /// Reduce for hot mics (e.g. 4.0); raise for quiet sources (e.g. 14.0).
    #[arg(long, env = "VOXTYPE_OSD_GAIN")]
    waveform_gain: Option<f32>,

    /// Panel look: "default" (filled waveform and peak meter) or "swiss"
    /// (time and word count, hairline waveform, status line, KDE Plasma
    /// colors).
    #[arg(long, env = "VOXTYPE_OSD_THEME", value_parser = parse_theme)]
    theme: Option<OsdTheme>,
}

fn parse_theme(s: &str) -> Result<OsdTheme, String> {
    OsdTheme::parse_str(s).ok_or_else(|| format!("unknown theme '{s}' (expected default or swiss)"))
}

/// State shared between the IPC worker and the GTK redraw timer.
struct SharedState {
    ring: Mutex<FrameRing>,
    window_frames: f64,
    peak: Mutex<PeakHold>,
    last_seq: Mutex<u64>,
    last_frame_at: Mutex<Instant>,
}

impl SharedState {
    fn new(decay_db_per_sec: f32, window_secs: f32) -> Self {
        let window_frames = window_secs.max(0.01) as f64 * FRAME_HZ as f64;
        Self {
            ring: Mutex::new(FrameRing::for_window(window_secs)),
            window_frames,
            peak: Mutex::new(PeakHold::new(decay_db_per_sec)),
            last_seq: Mutex::new(0),
            last_frame_at: Mutex::new(Instant::now() - Duration::from_secs(3600)),
        }
    }
}

fn main() -> anyhow::Result<()> {
    // Keep GDK off dmabuf rendering. The OSD is a small overlay;
    // shared-memory buffers cost nothing here. Respect an explicit
    // GDK_DISABLE from the user (they may be working around something else,
    // or testing the dmabuf path deliberately).
    if std::env::var_os("GDK_DISABLE").is_none() {
        std::env::set_var("GDK_DISABLE", "dmabuf");
    }

    // GDK_DISABLE=dmabuf does not stop GDK binding zwp_linux_dmabuf_v1 and
    // handling its feedback, which is where unfixed GTK releases munmap the
    // wrong pointer on output hotplug (gtk#8366; #580, #656). Only
    // GDK_WAYLAND_DISABLE keeps the interface unbound. Must run before GDK
    // opens the display.
    let (major, minor, micro) = (
        gtk4::major_version(),
        gtk4::minor_version(),
        gtk4::micro_version(),
    );
    let skip_dmabuf = gtk_env::gtk_lacks_dmabuf_feedback_fix(major, minor, micro);
    if skip_dmabuf {
        let existing = std::env::var("GDK_WAYLAND_DISABLE").ok();
        std::env::set_var(
            "GDK_WAYLAND_DISABLE",
            gtk_env::wayland_disable_with(existing.as_deref(), gtk_env::LINUX_DMABUF_INTERFACE),
        );
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if skip_dmabuf {
        tracing::info!(
            "GTK {major}.{minor}.{micro} predates the gtk#8366 fix; not binding {}",
            gtk_env::LINUX_DMABUF_INTERFACE
        );
    }

    let args = Args::parse();
    let socket_path = resolve_socket_path(args.socket.clone());

    // Layer config: defaults < config file [osd] < CLI/env overrides.
    let mut osd_cfg = load_osd_config_from_file(args.config.as_deref());
    if let Some(w) = args.width_px {
        osd_cfg.width_px = w;
    }
    if let Some(h) = args.height_px {
        osd_cfg.height_px = h;
    }
    if let Some(m) = args.margin_px {
        osd_cfg.margin_px = m;
    }
    if let Some(g) = args.waveform_gain {
        osd_cfg.waveform_gain = g;
    }
    if let Some(t) = args.theme {
        osd_cfg.theme = t;
    }
    // peak_decay_db_per_sec has a clap default value, so this always
    // overrides whatever the file said. That's intentional: if the user
    // passes the flag, honor it; if they don't, the clap default kicks in.
    osd_cfg.peak_decay_db_per_sec = args.peak_decay_db_per_sec;

    tracing::info!(
        "voxtype-osd-gtk4 starting; socket={:?} size={}x{} margin={} pos={:?} theme={:?}",
        socket_path,
        osd_cfg.width_px,
        osd_cfg.height_px,
        osd_cfg.margin_px,
        osd_cfg.position,
        osd_cfg.theme,
    );
    let stop_hint = stop_hint(&load_hotkey_config_from_file(args.config.as_deref()));

    let theme = ThemeWatcher::new();
    let mut palette = theme.palette();
    // [osd] opacity sets the panel background; the palette's own alpha
    // (0.85 in the fallback) used to win.
    palette.background.a = osd_cfg.opacity.clamp(0.0, 1.0);

    let state = Arc::new(SharedState::new(
        osd_cfg.peak_decay_db_per_sec,
        osd_cfg.waveform_window_secs,
    ));

    // Spawn the tokio IPC worker on a side thread.
    spawn_ipc_worker(
        state.clone(),
        socket_path,
        args.reconnect_secs,
        args.log_every,
    );

    // GTK application owns the main thread.
    let app = Application::builder().application_id(APP_ID).build();

    let cfg = osd_cfg.clone();
    let state_for_activate = state.clone();
    app.connect_activate(move |app| {
        build_window(app, &cfg, palette, &stop_hint, state_for_activate.clone());
    });

    // GTK's run() consumes argv; we've already parsed via clap, so feed
    // it an empty vector to keep it from re-parsing.
    let exit = app.run_with_args::<&str>(&[]);
    let code: u8 = exit.into();
    if code != 0 {
        anyhow::bail!("GTK application exited with status {}", code);
    }
    Ok(())
}

/// Spawn the tokio runtime + IPC loop on a dedicated thread.
fn spawn_ipc_worker(
    state: Arc<SharedState>,
    socket_path: PathBuf,
    reconnect_secs: f32,
    log_every: u32,
) {
    std::thread::Builder::new()
        .name("voxtype-osd-ipc".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("Failed to build tokio runtime: {e}");
                    return;
                }
            };

            let dt_per_frame = 1.0 / FRAME_HZ as f32;
            let mut total: u64 = 0;
            let mut last_log = Instant::now();

            let on_frame = move |frame: AudioFrame| {
                if let Ok(mut r) = state.ring.lock() {
                    r.push(frame);
                }
                if let Ok(mut p) = state.peak.lock() {
                    p.update(frame.peak_dbfs, dt_per_frame);
                }
                if let Ok(mut s) = state.last_seq.lock() {
                    *s = s.wrapping_add(1);
                }
                if let Ok(mut t) = state.last_frame_at.lock() {
                    *t = Instant::now();
                }

                total += 1;
                if log_every > 0 && total.is_multiple_of(u64::from(log_every)) {
                    let elapsed = last_log.elapsed().as_secs_f32();
                    let rate = if elapsed > 0.0 {
                        log_every as f32 / elapsed
                    } else {
                        0.0
                    };
                    tracing::debug!(
                        target: "osd::frames",
                        frontend = "gtk4",
                        seq = frame.seq,
                        peak_dbfs = frame.peak_dbfs,
                        min = frame.min,
                        max = frame.max,
                        rate_hz = rate,
                        "frame batch"
                    );
                    last_log = Instant::now();
                }
            };

            rt.block_on(run_ipc_loop(socket_path, reconnect_secs, on_frame));
        })
        .expect("spawn ipc worker thread");
}

/// Best-effort monitor height in physical pixels for translating the
/// fractional `top_margin` config into a layer-shell pixel offset.
///
/// Tries the GDK display's currently-focused monitor first (which is the
/// monitor the user is most likely looking at, and the one swayosd targets
/// via `--monitor`). If that fails — display unavailable, no monitors
/// enumerated — returns None and the caller falls back to a conservative
/// default.
fn focused_monitor_height_px() -> Option<i32> {
    use gtk4::gdk;
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    // `monitors` is a GListModel; pull the first item with a non-zero
    // height. On multi-monitor setups this picks whichever the compositor
    // ordered first, which lines up with the layer-shell default in
    // practice.
    for i in 0..monitors.n_items() {
        if let Some(obj) = monitors.item(i) {
            if let Ok(monitor) = obj.downcast::<gdk::Monitor>() {
                let h = monitor.geometry().height();
                if h > 0 {
                    return Some(h);
                }
            }
        }
    }
    None
}

/// Build the GTK window, attach layer-shell config, mount the DrawingArea,
/// and start the redraw tick.
fn build_window(
    app: &Application,
    cfg: &OsdConfig,
    palette: Palette,
    stop_hint: &str,
    state: Arc<SharedState>,
) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(cfg.width_px as i32)
        .default_height(cfg.height_px as i32)
        .resizable(false)
        .decorated(false)
        .build();

    // Layer-shell setup: top layer, no keyboard, anchored per config.
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_namespace(Some("voxtype-osd"));

    // Centered positions use swayosd-style fractional placement:
    // anchor only to Edge::Top with a margin of `top_margin × monitor_height`
    // and let the layer shell center horizontally. Matches what users
    // already see for volume/brightness/media-key feedback so the voxtype
    // OSD lands in the familiar band.
    //
    // Corner positions still use the absolute `margin_px` model — fractional
    // doesn't make sense there.
    let centered = matches!(
        cfg.position,
        OsdPosition::BottomCenter | OsdPosition::TopCenter
    );

    if centered {
        // Resolve monitor height to translate the fractional offset into
        // pixels. Falls back to a conservative 1080 if the display can't be
        // queried (extremely rare on Wayland-only systems where layer-shell
        // is supported at all).
        let monitor_height = focused_monitor_height_px().unwrap_or(1080);
        let top_px = (cfg.top_margin.clamp(0.0, 1.0) * monitor_height as f32) as i32;

        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, false);
        // Don't anchor Left/Right -> layer shell auto-centers horizontally.
        window.set_anchor(Edge::Left, false);
        window.set_anchor(Edge::Right, false);
        window.set_margin(Edge::Top, top_px);
    } else {
        // Corner positions: legacy anchor + uniform pixel margin behavior.
        let (anchor_top, anchor_bottom, anchor_left, anchor_right) = match cfg.position {
            OsdPosition::BottomLeft => (false, true, true, false),
            OsdPosition::BottomRight => (false, true, false, true),
            OsdPosition::TopLeft => (true, false, true, false),
            OsdPosition::TopRight => (true, false, false, true),
            // Centered branch is handled above; unreachable here.
            OsdPosition::BottomCenter | OsdPosition::TopCenter => unreachable!(),
        };
        window.set_anchor(Edge::Top, anchor_top);
        window.set_anchor(Edge::Bottom, anchor_bottom);
        window.set_anchor(Edge::Left, anchor_left);
        window.set_anchor(Edge::Right, anchor_right);

        let m = cfg.margin_px as i32;
        if anchor_top {
            window.set_margin(Edge::Top, m);
        }
        if anchor_bottom {
            window.set_margin(Edge::Bottom, m);
        }
        if anchor_left {
            window.set_margin(Edge::Left, m);
        }
        if anchor_right {
            window.set_margin(Edge::Right, m);
        }
    }

    // Don't reserve space on the output: the OSD floats over windows.
    window.set_exclusive_zone(0);

    // Live preview text (live_preview_model). Hidden while there is none,
    // so a plain recording looks as before.
    let preview = gtk4::Label::new(None);
    preview.set_wrap(true);
    preview.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    preview.set_xalign(0.0);
    preview.set_width_chars(1);
    preview.add_css_class("voxtype-preview");
    preview.set_visible(false);

    let chrome = match cfg.theme {
        OsdTheme::Default => {
            install_preview_css(&palette, cfg.preview_font_pt);
            // The drawing area fills the window.
            let drawing_area = DrawingArea::new();
            drawing_area.set_content_width(cfg.width_px as i32);
            drawing_area.set_content_height(cfg.height_px as i32);
            let state_for_draw = state.clone();
            let gain = cfg.waveform_gain as f64;
            drawing_area.set_draw_func(move |_area, cr, w, h| {
                draw(cr, w, h, &palette, &state_for_draw, gain);
            });
            preview.set_size_request(cfg.width_px as i32, -1);

            // Status of the final transcription after the recording stops: a
            // progress bar and a line such as "Finishing: 8 s of audio left".
            let status_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            status_box.add_css_class("voxtype-status");
            let status_bar = gtk4::ProgressBar::new();
            let status_label = gtk4::Label::new(None);
            status_label.set_xalign(0.0);
            status_box.append(&status_bar);
            status_box.append(&status_label);
            status_box.set_visible(false);

            let column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            column.append(&drawing_area);
            column.append(&preview);
            column.append(&status_box);
            window.set_child(Some(&column));
            redraw_every_frame(&drawing_area);
            Chrome::Default {
                status_box,
                status_bar,
                status_label,
            }
        }
        OsdTheme::Swiss => {
            let theme = plasma::load();
            install_swiss_css(&theme, cfg.opacity, cfg.preview_font_pt);
            let view = SwissView::build(cfg, &theme, stop_hint, &preview, state.clone());
            window.set_child(Some(&view.panel));
            Chrome::Swiss(view)
        }
    };

    // Click-through: install an empty input region on the window's surface
    // once it's realized. Until then `realize` hasn't allocated a surface.
    {
        let window_ref = window.clone();
        window.connect_realize(move |_| {
            apply_click_through(&window_ref);
        });
    }

    // Visibility must be polled separately: a hidden widget has no frame clock
    // ticks with which to notice the next recording.
    let redraw_state = state.clone();
    let redraw_window = window.clone();
    // Tracks GTK visibility. Starts true because `window.present()` below maps
    // the surface, the first tick's idle check then hides it.
    let visible = Cell::new(true);
    let ticks = Cell::new(0u32);
    let preview_path = voxtype::config::Config::live_preview_path();
    let status_path = voxtype::config::Config::live_status_path();
    // True while the daemon reports a final transcription in progress: the
    // audio frames have stopped, but the OSD stays up to show it.
    let finishing = Cell::new(false);
    // When the surface last appeared: the start of the recording.
    let shown_at = Cell::new(Instant::now());
    let default_size = (cfg.width_px as i32, cfg.height_px as i32);

    glib::timeout_add_local(Duration::from_millis(RENDER_TICK_MS as u64), move || {
        let cur_seq = redraw_state.last_seq.lock().map(|s| *s).unwrap_or(0);
        let last_at = redraw_state
            .last_frame_at
            .lock()
            .map(|t| *t)
            .unwrap_or_else(|_| Instant::now() - Duration::from_secs(3600));
        // About 10 Hz is plenty for text; the files live on tmpfs.
        ticks.set(ticks.get().wrapping_add(1));
        let poll = ticks.get().is_multiple_of(6);
        if poll {
            // A status older than 30 s is left over from a crash; ignore it.
            let fresh = std::fs::metadata(&status_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age < Duration::from_secs(30));
            finishing.set(fresh);
        }
        let idle = (last_at.elapsed().as_secs_f32() > IDLE_TIMEOUT_SECS && !finishing.get())
            || osd_suppressed();

        if idle {
            if visible.get() {
                tracing::info!("hiding (idle for {:.2}s)", last_at.elapsed().as_secs_f32());
                redraw_window.set_visible(false);
                visible.set(false);
                preview.set_text("");
                preview.set_visible(false);
                match &chrome {
                    Chrome::Default { status_box, .. } => status_box.set_visible(false),
                    Chrome::Swiss(view) => view.reset(),
                }
                // Shrink back: GTK keeps a grown window at its larger size.
                redraw_window.set_default_size(default_size.0, default_size.1);
            }
            return glib::ControlFlow::Continue;
        }

        if !visible.get() {
            shown_at.set(Instant::now());
        }

        if poll {
            let full = std::fs::read_to_string(&preview_path).unwrap_or_default();
            let text = preview_tail(full.trim());
            if preview.text() != text {
                preview.set_visible(!text.is_empty());
                preview.set_text(&text);
            }
            let status = std::fs::read_to_string(&status_path).unwrap_or_default();
            let (fraction, label) = status.split_once('\n').unwrap_or(("", ""));
            let fraction = fraction.trim().parse::<f64>().ok();
            match &chrome {
                Chrome::Default {
                    status_box,
                    status_bar,
                    status_label,
                } => {
                    match fraction {
                        Some(f) => status_bar.set_fraction(f.clamp(0.0, 1.0)),
                        // Unknown: the model is still loading.
                        None => status_bar.pulse(),
                    }
                    if status_label.text() != label {
                        status_label.set_text(label);
                    }
                    status_box.set_visible(!label.is_empty());
                }
                Chrome::Swiss(view) => {
                    // Frames stop at the end of the recording, so the clock
                    // stops there too.
                    let elapsed = last_at.saturating_duration_since(shown_at.get());
                    let words = full.split_whitespace().count();
                    view.update(elapsed, words, fraction, label);
                }
            }
        }

        if !visible.get() {
            tracing::info!(
                "showing (frame seq={}, last_at={:.3}s ago)",
                cur_seq,
                last_at.elapsed().as_secs_f32()
            );
            redraw_window.set_visible(true);
            visible.set(true);
        }

        // Decay the held peak even when no new frame arrived this tick.
        if let Ok(mut p) = redraw_state.peak.lock() {
            let dt = (RENDER_TICK_MS as f32) / 1000.0;
            // We pass the most recent peak from the ring as the "current"
            // value so a stale update doesn't snap the held value back up.
            let cur_peak = redraw_state
                .ring
                .lock()
                .ok()
                .and_then(|r| r.latest())
                .map(|f| f.peak_dbfs)
                .unwrap_or(-120.0);
            // Only decay; the IPC callback already snapped up on each
            // received frame. Calling update here with a non-louder peak
            // keeps the linear decay running at render rate.
            if cur_peak <= p.held_dbfs {
                p.update(cur_peak, dt);
            }
        }

        glib::ControlFlow::Continue
    });

    // Map the layer-shell surface once. The redraw timer will hide it
    // immediately on its first tick (no frames yet → idle), and toggle
    // visibility from there. Mapping once at startup keeps Hyprland's
    // layer-shell state machine happy across hide/show cycles.
    window.present();
}

/// Most characters of preview text to show. The tail is what you are saying
/// now; three lines at the default width hold about this much.
const PREVIEW_MAX_CHARS: usize = 160;

/// Keep the end of the running transcript, cut at a word boundary.
fn preview_tail(text: &str) -> String {
    let chars = text.chars().count();
    if chars <= PREVIEW_MAX_CHARS {
        return text.to_string();
    }
    let tail: String = text.chars().skip(chars - PREVIEW_MAX_CHARS).collect();
    match tail.split_once(' ') {
        Some((_, rest)) => format!("…{}", rest),
        None => format!("…{}", tail),
    }
}

/// Preview and status text on the OSD background.
fn install_preview_css(palette: &Palette, font_pt: Option<f32>) {
    let rgba = |c: &voxtype::osd::visual::Color, a: f32| {
        format!(
            "rgba({},{},{},{})",
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            a
        )
    };
    let font = font_pt
        .map(|pt| format!(" font-size: {}pt;", pt))
        .unwrap_or_default();
    let bg = rgba(&palette.background, palette.background.a);
    let css = format!(
        "window {{ background: transparent; }}\n\
         .voxtype-preview {{ background-color: {bg}; color: {}; padding: 6px 10px 8px 10px;{font} }}\n\
         .voxtype-status {{ background-color: {bg}; color: {}; padding: 0 10px 8px 10px; font-style: italic; }}\n\
         .voxtype-status progressbar {{ margin-bottom: 4px; }}",
        rgba(&palette.foreground, 1.0),
        rgba(&palette.foreground, 1.0),
    );
    add_css(&css);
}

fn add_css(css: &str) {
    let provider = gtk4::CssProvider::new();
    #[allow(deprecated)]
    provider.load_from_data(css);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Set an empty input region on the GdkSurface so clicks pass through.
fn apply_click_through(window: &ApplicationWindow) {
    let Some(surface) = window.surface() else {
        tracing::warn!("Window has no surface yet; click-through not applied");
        return;
    };
    let empty = Region::create_rectangle(&RectangleInt::new(0, 0, 0, 0));
    surface.set_input_region(Some(&empty));
}

/// Render the waveform + peak meter into the given Cairo context.
fn draw(
    cr: &Context,
    width: i32,
    height: i32,
    palette: &Palette,
    state: &Arc<SharedState>,
    gain: f64,
) {
    let w = width as f64;
    let h = height as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    // Clear background.
    cr.set_source_rgba(
        palette.background.r as f64,
        palette.background.g as f64,
        palette.background.b as f64,
        palette.background.a as f64,
    );
    cr.set_operator(cairo::Operator::Source);
    cr.paint().ok();
    cr.set_operator(cairo::Operator::Over);

    // Layout: waveform area on the left (~92% width), gap (1%), then peak
    // meter on the right (~7% width).
    let meter_width = (w * 0.07).max(8.0);
    let gap = (w * 0.01).max(2.0);
    let wave_width = (w - meter_width - gap).max(0.0);

    draw_waveform(
        cr,
        &Rect {
            x: 0.0,
            y: 0.0,
            w: wave_width,
            h,
        },
        palette,
        state,
        gain,
    );
    draw_peak_meter(
        cr,
        &Rect {
            x: wave_width + gap,
            y: 0.0,
            w: meter_width,
            h,
        },
        palette,
        state,
    );
}

/// Pixel-space rectangle for the draw helpers, so geometry travels as one
/// argument instead of four loose floats.
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn draw_waveform(cr: &Context, r: &Rect, palette: &Palette, state: &Arc<SharedState>, gain: f64) {
    let Rect { x, y, w, h } = *r;
    if w < 1.0 {
        return;
    }
    let n_columns = w.floor() as usize;
    if n_columns == 0 {
        return;
    }

    let cols = match state.ring.lock() {
        Ok(r) => {
            let frames: Vec<AudioFrame> = r.iter().collect();
            project_envelope(
                &frames,
                n_columns,
                state.window_frames,
                r.scroll_offset(Instant::now()),
            )
        }
        Err(_) => return,
    };

    let mid = y + h * 0.5;
    let half = h * 0.5;

    // Mirrored envelope filled polygon. We trace the top edge left-to-right
    // following `max`, then the bottom edge right-to-left following `min`.
    cr.set_source_rgba(
        palette.accent.r as f64,
        palette.accent.g as f64,
        palette.accent.b as f64,
        palette.accent.a as f64,
    );

    cr.new_path();
    // Top edge.
    for (i, col) in cols.iter().enumerate() {
        let px = x + i as f64 + 0.5;
        let py = mid - sample_to_pixels(col.max, half, gain);
        if i == 0 {
            cr.move_to(px, py);
        } else {
            cr.line_to(px, py);
        }
    }
    // Bottom edge, right-to-left.
    for (i, col) in cols.iter().enumerate().rev() {
        let px = x + i as f64 + 0.5;
        let py = mid - sample_to_pixels(col.min, half, gain);
        cr.line_to(px, py);
    }
    cr.close_path();
    cr.fill().ok();

    // Subtle centerline.
    cr.set_source_rgba(
        palette.foreground.r as f64,
        palette.foreground.g as f64,
        palette.foreground.b as f64,
        0.15,
    );
    cr.set_line_width(1.0);
    cr.move_to(x, mid);
    cr.line_to(x + w, mid);
    cr.stroke().ok();
}

fn sample_to_pixels(sample: f32, half_height: f64, gain: f64) -> f64 {
    (sample as f64 * gain).clamp(-1.0, 1.0) * half_height
}

fn draw_peak_meter(cr: &Context, r: &Rect, palette: &Palette, state: &Arc<SharedState>) {
    let Rect { x, y, w, h } = *r;
    if w < 1.0 || h < 1.0 {
        return;
    }

    let (latest_peak, held_peak) = {
        let latest = state
            .ring
            .lock()
            .ok()
            .and_then(|r| r.latest())
            .map(|f| f.peak_dbfs)
            .unwrap_or(f32::NEG_INFINITY);
        let held = state
            .peak
            .lock()
            .map(|p| p.held_dbfs)
            .unwrap_or(f32::NEG_INFINITY);
        (latest, held)
    };

    let fill_frac = peak_meter_fraction(latest_peak, METER_FLOOR_DBFS) as f64;

    let segments = METER_SEGMENTS;
    let gap = 1.5_f64;
    let total_gap = gap * (segments as f64 - 1.0);
    let seg_h = ((h - total_gap) / segments as f64).max(1.0);

    for i in 0..segments {
        // Segment 0 is the bottom of the bar.
        let frac_top = (i as f64 + 1.0) / segments as f64;
        let lit = fill_frac >= (i as f64 + 0.5) / segments as f64;
        // dBFS at the *top* of this segment for color zone classification.
        let seg_top_db = METER_FLOOR_DBFS + (frac_top as f32) * (-METER_FLOOR_DBFS);
        let zone = MeterZone::from_dbfs(seg_top_db);
        let zone_color = zone.color(palette);

        let py = y + h - (i as f64 + 1.0) * seg_h - i as f64 * gap;

        if lit {
            cr.set_source_rgba(
                zone_color.r as f64,
                zone_color.g as f64,
                zone_color.b as f64,
                zone_color.a as f64,
            );
        } else {
            cr.set_source_rgba(
                zone_color.r as f64,
                zone_color.g as f64,
                zone_color.b as f64,
                0.18,
            );
        }
        cr.rectangle(x, py, w, seg_h);
        cr.fill().ok();
    }

    // Held-peak tick (1.5 px line at the held position).
    if held_peak.is_finite() && held_peak > METER_FLOOR_DBFS {
        let held_frac = peak_meter_fraction(held_peak, METER_FLOOR_DBFS) as f64;
        let py = y + h - held_frac * h;
        cr.set_source_rgba(
            palette.foreground.r as f64,
            palette.foreground.g as f64,
            palette.foreground.b as f64,
            0.95,
        );
        cr.set_line_width(1.5);
        cr.move_to(x, py);
        cr.line_to(x + w, py);
        cr.stroke().ok();
    }
}

/// Ask for a redraw on every frame of the GTK frame clock. GTK pauses tick
/// callbacks while the widget is hidden; while visible, this keeps the
/// waveform scrolling between audio deliveries.
fn redraw_every_frame(area: &DrawingArea) {
    area.add_tick_callback(|area, _clock| {
        area.queue_draw();
        glib::ControlFlow::Continue
    });
}

/// The widgets around the preview text that differ between themes.
enum Chrome {
    Default {
        status_box: gtk4::Box,
        status_bar: gtk4::ProgressBar,
        status_label: gtk4::Label,
    },
    Swiss(SwissView),
}

/// Waveform tick spacing in the `swiss` theme, in pixels.
const SWISS_TICK_SPACING_PX: f64 = 4.0;

/// Height of the `swiss` waveform strip.
const SWISS_WAVE_HEIGHT_PX: i32 = 40;

/// Transcript size in the `swiss` theme when `preview_font_pt` is unset.
const SWISS_TEXT_PT: f32 = 18.0;

/// The `swiss` panel: a label row with elapsed time and word count, a
/// hairline waveform, the transcript, and a status line with a thin
/// progress bar while the final text is computed.
struct SwissView {
    panel: gtk4::Box,
    time: gtk4::Label,
    words: gtk4::Label,
    dot: gtk4::Box,
    state_label: gtk4::Label,
    detail: gtk4::Label,
    progress: DrawingArea,
    /// Progress of the final transcription; `None` while it is unknown.
    fraction: Rc<Cell<Option<f64>>>,
    /// False after the recording stops: the waveform then drops the accent.
    live: Rc<Cell<bool>>,
    /// "tap Right Ctrl to stop", or empty when voxtype has no hotkey.
    listening_detail: String,
}

impl SwissView {
    fn build(
        cfg: &OsdConfig,
        theme: &PlasmaTheme,
        stop_hint: &str,
        preview: &gtk4::Label,
        state: Arc<SharedState>,
    ) -> Self {
        let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        panel.add_css_class("swiss-panel");
        panel.set_size_request(cfg.width_px as i32, -1);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        let icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
        icon.set_pixel_size(14);
        icon.add_css_class("swiss-accent");
        icon.set_valign(gtk4::Align::Start);
        let title = caption_label("VOICE DICTATION", "swiss-caption");
        title.set_valign(gtk4::Align::Start);
        title.set_margin_start(8);
        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let (time_box, time) = stat("00:00", "DURATION");
        let (words_box, words) = stat("0", "WORDS");
        let rule = gtk4::Separator::new(gtk4::Orientation::Vertical);
        rule.add_css_class("swiss-rule");
        rule.set_margin_start(16);
        rule.set_margin_end(16);
        header.append(&icon);
        header.append(&title);
        header.append(&spacer);
        header.append(&time_box);
        header.append(&rule);
        header.append(&words_box);

        let wave = DrawingArea::new();
        wave.set_content_height(SWISS_WAVE_HEIGHT_PX);
        wave.set_margin_top(10);
        wave.set_margin_bottom(10);
        let colors = (theme.inactive, theme.accent, theme.border);
        let gain = cfg.waveform_gain as f64;
        let live = Rc::new(Cell::new(true));
        let live_for_draw = live.clone();
        wave.set_draw_func(move |_area, cr, w, h| {
            let live = live_for_draw.get();
            draw_swiss_wave(cr, w as f64, h as f64, colors, &state, gain, live);
        });
        redraw_every_frame(&wave);

        preview.add_css_class("swiss-text");
        preview.set_margin_bottom(4);

        let bottom_rule = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        bottom_rule.add_css_class("swiss-rule");
        bottom_rule.set_margin_top(12);

        let status = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        status.set_margin_top(10);
        let dot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        dot.add_css_class("swiss-dot");
        dot.set_valign(gtk4::Align::Center);
        let state_label = caption_label("Listening", "swiss-state");
        state_label.set_margin_start(10);
        let detail = caption_label("", "swiss-detail");
        detail.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        let fraction = Rc::new(Cell::new(None));
        let progress = DrawingArea::new();
        progress.set_content_height(2);
        progress.set_hexpand(true);
        progress.set_valign(gtk4::Align::Center);
        progress.set_margin_start(16);
        let frac = fraction.clone();
        let (track, fill) = (theme.border, theme.accent);
        let started = Instant::now();
        progress.set_draw_func(move |_area, cr, w, h| {
            draw_hairline_progress(cr, w as f64, h as f64, track, fill, frac.get(), started);
        });
        redraw_every_frame(&progress);
        progress.set_visible(false);
        status.append(&dot);
        status.append(&state_label);
        status.append(&detail);
        status.append(&progress);

        panel.append(&header);
        panel.append(&wave);
        panel.append(preview);
        panel.append(&bottom_rule);
        panel.append(&status);

        let listening_detail = if stop_hint.is_empty() {
            String::new()
        } else {
            format!("  ·  {stop_hint}")
        };
        let view = Self {
            panel,
            time,
            words,
            dot,
            state_label,
            detail,
            progress,
            fraction,
            live,
            listening_detail,
        };
        view.reset();
        view
    }

    /// Back to the start-of-recording state, ready for the next one.
    fn reset(&self) {
        self.time.set_text("00:00");
        self.words.set_text("0");
        self.show_status("");
        self.fraction.set(None);
    }

    fn update(&self, elapsed: Duration, words: usize, fraction: Option<f64>, status: &str) {
        let secs = elapsed.as_secs();
        set_text_if_changed(&self.time, &format!("{:02}:{:02}", secs / 60, secs % 60));
        set_text_if_changed(&self.words, &words.to_string());
        self.fraction.set(fraction.map(|f| f.clamp(0.0, 1.0)));
        self.show_status(status);
    }

    /// `status` is the daemon's line such as "Finishing: 4 s of audio
    /// left", or empty while recording.
    fn show_status(&self, status: &str) {
        let (state, detail) = if status.is_empty() {
            ("Listening".to_string(), self.listening_detail.clone())
        } else {
            match status.split_once(": ") {
                Some((head, rest)) => (head.to_string(), format!("  ·  {rest}")),
                None => (status.to_string(), String::new()),
            }
        };
        set_text_if_changed(&self.state_label, &state);
        set_text_if_changed(&self.detail, &detail);
        let finishing = !status.is_empty();
        self.progress.set_visible(finishing);
        self.live.set(!finishing);
        if finishing {
            self.dot.add_css_class("finishing");
        } else {
            self.dot.remove_css_class("finishing");
        }
    }
}

fn set_text_if_changed(label: &gtk4::Label, text: &str) {
    if label.text() != text {
        label.set_text(text);
    }
}

fn caption_label(text: &str, class: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class(class);
    label
}

/// A value over a small caption, such as "00:12" over "DURATION".
fn stat(value: &str, caption: &str) -> (gtk4::Box, gtk4::Label) {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let value = caption_label(value, "swiss-value");
    column.append(&value);
    column.append(&caption_label(caption, "swiss-caption-small"));
    (column, value)
}

/// CSS for the `swiss` panel. The window stays transparent so the 1 px
/// border and the slightly rounded corners show.
fn install_swiss_css(theme: &PlasmaTheme, opacity: f32, font_pt: Option<f32>) {
    let rgba = |c: Color, a: f32| {
        format!(
            "rgba({},{},{},{})",
            (c.r * 255.0).round() as u8,
            (c.g * 255.0).round() as u8,
            (c.b * 255.0).round() as u8,
            a
        )
    };
    let bg = rgba(theme.background, opacity.clamp(0.0, 1.0));
    let fg = rgba(theme.foreground, 1.0);
    let dim = rgba(theme.inactive, 1.0);
    let accent = rgba(theme.accent, 1.0);
    let border = rgba(theme.border, 1.0);
    let sans = match &theme.font_family {
        Some(f) => format!("\"{f}\", sans-serif"),
        None => "sans-serif".to_string(),
    };
    let mono = match &theme.mono_family {
        Some(f) => format!("\"{f}\", monospace"),
        None => "monospace".to_string(),
    };
    let text_pt = font_pt.unwrap_or(SWISS_TEXT_PT);
    let css = format!(
        "window {{ background: transparent; }}\n\
         .swiss-panel {{ background-color: {bg}; border: 1px solid {border}; border-radius: 3px; \
           padding: 16px 22px 12px 22px; }}\n\
         .swiss-caption {{ font-family: {mono}; font-size: 8.5pt; letter-spacing: 2px; color: {dim}; }}\n\
         .swiss-caption-small {{ font-family: {mono}; font-size: 7pt; letter-spacing: 1.5px; color: {dim}; }}\n\
         .swiss-value {{ font-family: {mono}; font-size: 11pt; color: {fg}; font-feature-settings: \"tnum\"; }}\n\
         .swiss-accent {{ color: {accent}; }}\n\
         separator.swiss-rule {{ background: {border}; min-width: 1px; min-height: 1px; margin-top: 0; margin-bottom: 0; }}\n\
         .swiss-text {{ font-family: {sans}; font-size: {text_pt}pt; font-weight: 500; color: {fg}; letter-spacing: -0.2px; }}\n\
         .swiss-state {{ font-family: {mono}; font-size: 9pt; color: {accent}; }}\n\
         .swiss-detail {{ font-family: {mono}; font-size: 9pt; color: {dim}; }}\n\
         .swiss-dot {{ min-width: 7px; min-height: 7px; background-color: {accent}; }}\n\
         .swiss-dot.finishing {{ background-color: transparent; border: 1px solid {accent}; min-width: 5px; min-height: 5px; }}"
    );
    add_css(&css);
}

/// Hairline waveform: one thin tick per few frames, mirrored on a 1 px
/// baseline. Ticks are tied to frame numbers so they scroll with the audio
/// instead of shimmering in place. Louder ticks shift from the inactive
/// color to the accent, which stands in for a separate level meter. Once
/// the recording stops (`live` false) the frozen waveform stays inactive.
fn draw_swiss_wave(
    cr: &Context,
    w: f64,
    h: f64,
    (quiet, loud, baseline): (Color, Color, Color),
    state: &Arc<SharedState>,
    gain: f64,
    live: bool,
) {
    if w < 1.0 || h < 1.0 {
        return;
    }
    let mid = (h * 0.5).floor() + 0.5;
    cr.set_line_width(1.0);
    set_source(cr, baseline, 1.0);
    cr.move_to(0.0, mid);
    cr.line_to(w, mid);
    cr.stroke().ok();

    let (frames, offset) = match state.ring.lock() {
        Ok(r) => (
            r.iter().collect::<Vec<_>>(),
            r.scroll_offset(Instant::now()),
        ),
        Err(_) => return,
    };
    if frames.is_empty() {
        return;
    }
    let px_per_frame = w / state.window_frames;
    let every = (SWISS_TICK_SPACING_PX / px_per_frame).ceil().max(1.0) as usize;
    let right = (frames.len() - 1) as f64 + offset;
    let max_half = h * 0.5 - 1.0;
    let fade_px = w * 0.2;

    for (i, frame) in frames.iter().enumerate() {
        if !(frame.seq as usize).is_multiple_of(every) {
            continue;
        }
        let x = (w - 1.0 - (right - i as f64) * px_per_frame).round() + 0.5;
        if x < 0.0 || x > w {
            continue;
        }
        let amp = frames[i.saturating_sub(every - 1)..=i]
            .iter()
            .map(|f| f.max.max(-f.min))
            .fold(0.0_f32, f32::max) as f64;
        let level = tick_level(amp, gain);
        let half = (level * max_half).max(1.0);
        let t = if live {
            smoothstep(0.3, 0.65, level) as f32
        } else {
            0.0
        };
        let color = Color::rgb(
            quiet.r + (loud.r - quiet.r) * t,
            quiet.g + (loud.g - quiet.g) * t,
            quiet.b + (loud.b - quiet.b) * t,
        );
        // Older audio fades out toward the left edge.
        let alpha = (x / fade_px).min(1.0) * (0.55 + 0.45 * t as f64);
        set_source(cr, color, alpha);
        cr.move_to(x, mid - half);
        cr.line_to(x, mid + half);
        cr.stroke().ok();
    }
}

/// Tick height, 0.0..=1.0, from a sample peak. Uses a -60..0 dBFS scale
/// raised to 1.5, so speech fills the strip and room noise stays near the line.
/// `waveform_gain` scales relative to its default of 10.
fn tick_level(amp: f64, gain: f64) -> f64 {
    let amp = amp * gain / 10.0;
    if amp <= 0.0 {
        return 0.0;
    }
    let dbfs = 20.0 * amp.log10();
    let level = peak_meter_fraction(dbfs as f32, METER_FLOOR_DBFS) as f64;
    level.powf(1.5)
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A 1 px progress line. With no fraction (the model is still loading) a
/// short segment sweeps across instead.
fn draw_hairline_progress(
    cr: &Context,
    w: f64,
    h: f64,
    track: Color,
    fill: Color,
    fraction: Option<f64>,
    started: Instant,
) {
    if w < 1.0 || h < 1.0 {
        return;
    }
    set_source(cr, track, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    cr.fill().ok();
    set_source(cr, fill, 1.0);
    match fraction {
        Some(f) => cr.rectangle(0.0, 0.0, (w * f).round(), h),
        None => {
            let seg = w * 0.25;
            let phase = started.elapsed().as_secs_f64() % 1.6 / 1.6;
            let x = phase * (w + seg) - seg;
            cr.rectangle(x.max(0.0), 0.0, (x + seg).min(w) - x.max(0.0), h);
        }
    }
    cr.fill().ok();
}

fn set_source(cr: &Context, c: Color, alpha: f64) {
    cr.set_source_rgba(c.r as f64, c.g as f64, c.b as f64, alpha);
}

/// "tap Right Ctrl to stop" from `[hotkey]`, or empty when the hotkey is
/// off (compositor bindings call `voxtype record` instead).
fn stop_hint(hotkey: &HotkeyConfig) -> String {
    if !hotkey.enabled {
        return String::new();
    }
    let key = hotkey
        .modifiers
        .iter()
        .chain(std::iter::once(&hotkey.key))
        .map(|k| key_label(k))
        .collect::<Vec<_>>()
        .join("+");
    match hotkey.mode {
        ActivationMode::PushToTalk => format!("release {key} to stop"),
        ActivationMode::Toggle if hotkey.double_tap => format!("double tap {key} to stop"),
        ActivationMode::Toggle => format!("tap {key} to stop"),
    }
}

/// "RIGHTCTRL" -> "Right Ctrl", "KEY_SCROLLLOCK" -> "Scrolllock".
fn key_label(name: &str) -> String {
    let upper = name.trim().to_ascii_uppercase();
    let upper = upper.strip_prefix("KEY_").unwrap_or(&upper);
    let (side, rest) = match (upper.strip_prefix("RIGHT"), upper.strip_prefix("LEFT")) {
        (Some(r), _) if !r.is_empty() => ("Right ", r),
        (_, Some(r)) if !r.is_empty() => ("Left ", r),
        _ => ("", upper),
    };
    let rest = match rest {
        "CTRL" | "CONTROL" => "Ctrl".to_string(),
        "META" | "SUPER" => "Super".to_string(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_string() + &chars.as_str().to_ascii_lowercase())
                .unwrap_or_default()
        }
    };
    format!("{side}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_labels_read_like_keycaps() {
        assert_eq!(key_label("RIGHTCTRL"), "Right Ctrl");
        assert_eq!(key_label("KEY_LEFTALT"), "Left Alt");
        assert_eq!(key_label("SCROLLLOCK"), "Scrolllock");
        assert_eq!(key_label("RIGHT"), "Right");
        assert_eq!(key_label("leftmeta"), "Left Super");
    }

    #[test]
    fn stop_hint_follows_mode() {
        let mut hk = HotkeyConfig {
            key: "RIGHTCTRL".into(),
            ..HotkeyConfig::default()
        };
        hk.mode = ActivationMode::PushToTalk;
        assert_eq!(stop_hint(&hk), "release Right Ctrl to stop");
        hk.mode = ActivationMode::Toggle;
        assert_eq!(stop_hint(&hk), "tap Right Ctrl to stop");
        hk.double_tap = true;
        assert_eq!(stop_hint(&hk), "double tap Right Ctrl to stop");
        hk.enabled = false;
        assert_eq!(stop_hint(&hk), "");
    }

    #[test]
    fn tick_level_maps_speech_high_and_silence_low() {
        assert_eq!(tick_level(0.0, 10.0), 0.0);
        assert!(tick_level(0.001, 10.0) < 0.05);
        assert!(tick_level(0.3, 10.0) > 0.6);
        assert!(tick_level(0.03, 20.0) > tick_level(0.03, 10.0));
    }
}
