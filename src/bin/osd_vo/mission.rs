//! The `mission` look of the vo popup, after mock-up 22: a mission-control
//! telemetry panel. A row of tiles for the model, the GPU memory, the
//! latency, the microphone and the elapsed time, a large transcript tile
//! with orange corner brackets, a waveform strip with a LIVE meter, and a
//! status bar, orange and white on dark slate.
//!
//! Five tiles do not fit in 600 px with type that can be read, so the panel
//! is wider than the other looks: 760 px at the default width.
//!
//! While the popup is open it keeps a little idle life (n7b, RD-02). The
//! latency tile's level meter ticks at 4 Hz with the real level, as
//! telemetry does. The status mark's ring turns into a radar: a soft
//! sweep goes round inside it once every 4 s. The elapsed tile's three dots
//! act as status LEDs and blink at 0.5, 0.8 and 1.3 Hz between 40 % and
//! 100 %. All step at 15 frames a second, stop once the popup closes and
//! stay still under reduced motion.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::kit::{at, ease_out, facts, hex, mix, reduced, Levels, Mode};
use super::*;

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo::scene::lattice::LatticeStyle;

/// Colours sampled from the mock-up.
const ORANGE: u32 = 0xFF7A1A;
const SLATE: u32 = 0x0D1117;
const TILE: u32 = 0x111821;
const TILE_EDGE: u32 = 0x2A3340;
const INK: u32 = 0xF2F4F7;
const LABEL: u32 = 0x9AA4B2;
const SOFT: u32 = 0x7D8796;
const TRACK: u32 = 0x232B36;
const GREY: u32 = 0x5E6672;

/// The panel is this much wider than the configured popup.
const WIDEN: f32 = 760.0 / 600.0;
/// The narrowest configured width the panel is laid out for. The tiles are
/// drawn for 600 px; a config with no `[osd]` section falls back to 400 px,
/// which squeezed them until their labels truncated.
const MIN_WIDTH: f32 = 600.0;
const RADIUS: f32 = 10.0;
const PAD: f32 = 13.0;
const PAD_BOTTOM: f32 = 8.0;
const GAP: f32 = 10.0;
/// The telemetry row: five tiles with these relative widths.
const TILES_H: f32 = 72.0;
const TILE_GAP: f32 = 9.0;
const TILE_W: [f32; 5] = [226.0, 195.0, 172.0, 197.0, 145.0];
/// Where the model text starts in its tile, and the room kept right of it.
const MODEL_X: f32 = 38.0;
const TILE_RIGHT: f32 = 10.0;
/// The transcript tile.
const LABEL_Y: f32 = 15.0;
const TEXT_X: f32 = 23.0;
/// Room kept free right of the words.
const TEXT_RIGHT: f32 = 27.0;
const TEXT_TOP: f32 = 35.0;
const TEXT_BOTTOM: f32 = 13.0;
const TEXT_PX: f32 = 38.4;
const TEXT_LINE: f32 = 44.0;
const MAX_LINES: u32 = 5;
/// Room left of the word count for the notch its casts land in.
const NOTCH_ROOM: f32 = 8.0;
/// The waveform strip.
const WAVE_H: f32 = 71.0;
const WAVE_HALF: f32 = 27.0;
const BAR_PITCH: f32 = 5.5;
const BAR_W: f32 = 2.0;
/// Where the newest audio sits along the strip; right of it the line waits.
const PLAYHEAD: f32 = 0.62;
const METER_W: f32 = 56.0;
const STATUS_H: f32 = 44.0;

#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    ui: u32,
    bold: u32,
    caps: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Inter:bold", SANS),
            ui: add("Inter", SANS),
            bold: add("Inter:semibold", SANS),
            caps: add("Barlow:medium", SANS),
        }
    })
}

fn panel_w(look: &Look) -> f32 {
    (look.width.max(MIN_WIDTH) * WIDEN).round()
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(-0.9)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    panel_w(look) - 2.0 * PAD - TEXT_X - TEXT_RIGHT
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

fn transcript_h(lines: u32) -> f32 {
    TEXT_TOP + lines as f32 * TEXT_LINE + TEXT_BOTTOM
}

fn panel_h(lines: u32) -> f32 {
    PAD + TILES_H + GAP + transcript_h(lines) + GAP + WAVE_H + GAP + STATUS_H + PAD_BOTTOM
}

pub(crate) fn surface(look: &Look) -> Size {
    Size::new(
        panel_w(look) + 24.0,
        (panel_h(MAX_LINES) + lattice_room(look) + 24.0).ceil(),
    )
}

/// LT-07: with `kinetic-lattice` the transcript tile grows by this under the
/// newest line, the gutter its strands use; without it, 0.
const LATTICE_GUTTER: f32 = 22.0;

fn lattice_room(look: &Look) -> f32 {
    if look.on(wild_space::KINETIC_LATTICE) {
        LATTICE_GUTTER
    } else {
        0.0
    }
}

/// GPU memory in use and in all, in GB, read at most every three seconds
/// and only while the popup asks for it. AMD cards report it in sysfs;
/// for others `nvidia-smi` is asked on a thread of its own.
fn vram() -> Option<(f32, f32)> {
    struct Cache {
        at: Option<Instant>,
        busy: bool,
        value: Option<(f32, f32)>,
    }
    static CACHE: Mutex<Cache> = Mutex::new(Cache {
        at: None,
        busy: false,
        value: None,
    });
    let mut c = CACHE.lock().ok()?;
    let stale = c.at.is_none_or(|a| a.elapsed() > Duration::from_secs(3));
    if stale && !c.busy {
        c.busy = true;
        std::thread::spawn(|| {
            let value = read_vram();
            if let Ok(mut c) = CACHE.lock() {
                c.value = value;
                c.at = Some(Instant::now());
                c.busy = false;
            }
        });
    }
    c.value
}

fn read_vram() -> Option<(f32, f32)> {
    const GB: f32 = 1024.0 * 1024.0 * 1024.0;
    let num = |p: &std::path::Path| -> Option<f32> {
        std::fs::read_to_string(p).ok()?.trim().parse::<f64>().ok().map(|v| v as f32)
    };
    for card in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let dev = card.path().join("device");
        if let (Some(used), Some(total)) = (
            num(&dev.join("mem_info_vram_used")),
            num(&dev.join("mem_info_vram_total")),
        ) {
            if total > 0.0 {
                return Some((used / GB, total / GB));
            }
        }
    }
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.lines().next()?.split(',');
    let used: f32 = parts.next()?.trim().parse().ok()?;
    let total: f32 = parts.next()?.trim().parse().ok()?;
    (total > 0.0).then_some((used / 1024.0, total / 1024.0))
}

/// `vo::scene::roll::digit_roll` (counting up) with each digit's ten figures
/// in one label of ten lines instead of ten labels. It draws the same, and
/// the view, rebuilt every frame on room tone, has two nodes per digit
/// instead of twelve: the tiles' eleven rolling digits were about a sixth
/// of mission's idle CPU.
fn digit_roll(
    key: &str,
    text: &str,
    style: TextStyle,
    color: Color,
    spring: Spring,
) -> Element<Msg> {
    const FIGURES: &str = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9";
    let lh = style.line_height;
    let n = text.chars().count();
    let cells: Vec<Element<Msg>> = text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            // Keyed by place from the right, and a column by the text to its
            // left, so a carry rolls a new column in as in `digit_roll`.
            let place = n - 1 - i;
            match c.to_digit(10) {
                Some(d) => {
                    let prefix: String = text.chars().take(i).collect();
                    let figures = Element::label(format!("{key}-c{place}-{prefix}"), FIGURES)
                        .text_style(style)
                        .color(color)
                        .height(10.0 * lh)
                        .shrink(0.0)
                        .offset(0.0, -(d as f32) * lh)
                        .motion(Prop::Offset, spring)
                        .presence(
                            Transition::NONE.offset(0.0, lh),
                            Transition::NONE
                                .offset(0.0, -(d as f32 + 1.0) * lh)
                                .opacity(0.0),
                        )
                        .motion(Prop::Opacity, Spring::SNAP);
                    Element::layer(format!("{key}-d{place}"))
                        .height(lh)
                        .align_items(AlignItems::FLEX_START)
                        .fade_edges(0.0, 0.2 * lh, 0.0, 0.2 * lh)
                        .child(figures)
                }
                None if c.is_whitespace() => Element::row(format!("{key}-s{place}"))
                    .width(0.26 * style.size)
                    .height(lh),
                None => Element::label(format!("{key}-s{place}"), c.to_string())
                    .text_style(style)
                    .color(color)
                    .height(lh),
            }
        })
        .collect();
    Element::row(key.to_string()).children(cells)
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// A line icon on the 24 unit grid.
fn icon(key: &'static str, outline: Arc<Outline>, px: f32, color: Color) -> Element<Msg> {
    Element::stroke_path(
        key,
        outline,
        Rect::new(0.0, 0.0, 24.0, 24.0),
        StrokeStyle::solid(1.8, color),
    )
    .size(px, px)
}

/// The tile icons, flattened once rather than on every view.
struct Icons {
    model: Arc<Outline>,
    chip: Arc<Outline>,
    timer: Arc<Outline>,
    mic: Arc<Outline>,
}

fn tile_icons() -> &'static Icons {
    static ICONS: OnceLock<Icons> = OnceLock::new();
    ICONS.get_or_init(|| Icons {
        model: Arc::new(model_icon()),
        chip: Arc::new(Outline::new(&icons::chip(), 0.05)),
        timer: Arc::new(Outline::new(&icons::timer(), 0.05)),
        mic: Arc::new(Outline::new(&icons::mic(), 0.05)),
    })
}

/// A small network: three nodes joined, for the model.
fn model_icon() -> Outline {
    let a = Point::new(6.0, 7.0);
    let b = Point::new(18.0, 9.0);
    let c = Point::new(11.0, 18.0);
    let mut contours = vec![Contour::polyline(&[a, b, c, a], true)];
    for p in [a, b, c] {
        contours.push(make::ring(p, 2.4));
    }
    Outline { contours }
}

/// Corner marks of a tile: short L shapes at the chosen corners.
#[derive(Clone, Copy)]
struct Corners {
    len: f32,
    width: f32,
    /// Top left, top right, bottom right, bottom left.
    which: [bool; 4],
    color: Color,
}

impl Corners {
    fn draw(&self, r: Rect, p: &mut Painter) {
        let (l, i) = (self.len, self.width / 2.0);
        let (x0, y0, x1, y1) = (r.x + i, r.y + i, r.x + r.w - i, r.y + r.h - i);
        let marks = [
            [Point::new(x0, y0 + l), Point::new(x0, y0), Point::new(x0 + l, y0)],
            [Point::new(x1 - l, y0), Point::new(x1, y0), Point::new(x1, y0 + l)],
            [Point::new(x1, y1 - l), Point::new(x1, y1), Point::new(x1 - l, y1)],
            [Point::new(x0 + l, y1), Point::new(x0, y1), Point::new(x0, y1 - l)],
        ];
        for (m, on) in marks.iter().zip(self.which) {
            if on {
                p.stroke(
                    &Contour::polyline(m, false),
                    &StrokeStyle::solid(self.width, self.color),
                );
            }
        }
    }
}

fn corners(key: &'static str, w: f32, h: f32, c: Corners) -> Element<Msg> {
    at(
        Element::canvas(key, 0.0, move |_, r, p| c.draw(r, p)).size(w, h),
        0.0,
        0.0,
    )
}

/// The waveform strip: bars scroll left from the play head, a dotted line
/// waits right of it, and a LIVE meter stands at the right end.
#[derive(Clone)]
struct Wave {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    progress: Option<f32>,
    /// Seconds of audio the popup keeps; the bars span them.
    window: f64,
}

impl Wave {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing => f64::INFINITY,
            _ => self.mode_at + 0.6,
        }
    }

    fn level(&self, s: f64, dt: f64) -> f32 {
        let jitter = 0.55 + 0.45 * noise((s / dt).round() as i64 as u32);
        self.levels.at(s) * jitter
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let cy = r.y + r.h / 2.0;
        let x0 = r.x + 20.0;
        let x1 = r.x + r.w - METER_W - 12.0;
        let head = x0 + (x1 - x0) * PLAYHEAD;
        let open = if reduced() {
            1.0
        } else {
            ease_out((t - self.shown_at - 0.15) / 0.35)
        };
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let orange = hex(ORANGE);
        let fade = match self.mode {
            Mode::Cancelled => 0.35,
            Mode::Done => 1.0 - 0.4 * ease_out((t - self.mode_at) / 0.4),
            _ => 1.0,
        };
        // The dotted line right of the play head, and at the far left.
        let dot = |p: &mut Painter, x: f32, a: f32| {
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x - 1.2, cy - 0.6, 2.4, 1.2), 0.6),
                Paint::solid(hex(LABEL).fade(a * fade)),
            ));
        };
        let mut x = head + BAR_PITCH;
        while x < x1 {
            if (x - head) / (x1 - head) <= open {
                dot(p, x, 0.45);
            }
            x += BAR_PITCH;
        }
        let n = ((head - x0) / BAR_PITCH) as i32;
        let dt = self.window / n.max(1) as f64;
        for i in 0..=n {
            let x = head - i as f32 * BAR_PITCH;
            let reach = i as f32 / n as f32;
            if reach > open {
                continue;
            }
            let at = t_eff - i as f64 * dt;
            let taper = (1.0 - reach.powf(3.0)).max(0.0);
            let half = (self.level(at, dt) * 1.3).min(1.0) * taper * WAVE_HALF;
            // Orange fills from the left as the final pass goes; the rest
            // waits in grey.
            let along = (x - x0) / (head - x0);
            let c = match (self.mode, self.progress) {
                (Mode::Finishing, Some(f)) if along > f => hex(GREY),
                (Mode::Finishing, None) => hex(GREY),
                (Mode::Cancelled, _) => hex(GREY),
                _ => mix(hex(INK), orange, i as f32 / 7.0),
            }
            .fade(fade);
            if half < 1.5 {
                dot(p, x, 0.55);
                continue;
            }
            let bar = Rect::new(x - BAR_W / 2.0, cy - half, BAR_W, 2.0 * half);
            if half > WAVE_HALF * 0.35 && self.mode == Mode::Listening {
                p.shape(ShapeInstance::rrect(
                    RRect::new(bar, BAR_W / 2.0),
                    Paint::glow(c.fade(0.2), 4.0, 0.4),
                ));
            }
            p.shape(ShapeInstance::rrect(RRect::new(bar, BAR_W / 2.0), Paint::solid(c)));
        }
        // The meter: a rule, then six segments that light with the level,
        // with a peak that falls a segment per 150 ms.
        let mx = r.x + r.w - METER_W;
        p.shape(ShapeInstance::line(
            Point::new(mx, r.y + 9.0),
            Point::new(mx, r.y + r.h - 9.0),
            1.0,
            Paint::solid(hex(TILE_EDGE)),
        ));
        let live = matches!(self.mode, Mode::Listening);
        let lit = |s: f64| (self.levels.at(s) * 1.5).min(1.0) * 6.0;
        let (now, peak) = if live {
            let peak = (0..12)
                .map(|k| lit(t - k as f64 * 0.05) - k as f32 * 0.05 / 0.15)
                .fold(0.0f32, f32::max);
            (lit(t), peak)
        } else {
            (0.0, 0.0)
        };
        let (sw, sh, sg) = (11.0, 3.6, 2.4);
        let sx = mx + (METER_W - sw) / 2.0;
        for k in 0..6 {
            let y = r.y + 11.0 + (5 - k) as f32 * (sh + sg);
            let on = (k as f32) < now.round();
            let held = (k as f32 - peak.round() + 1.0).abs() < 0.5;
            let c = if on || held {
                orange
            } else {
                hex(TRACK)
            };
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(sx, y, sw, sh), 0.8),
                Paint::solid(c.fade(fade)),
            ));
        }
    }
}

/// n7b: the idle motion's pace (RD-02's slow-loop cap), the telemetry
/// meter's rate, the radar's turn, its sweep's alpha and half width in
/// turns, and the three LEDs' rates and dimmest alpha.
const IDLE_FPS: f64 = 15.0;
const TELEMETRY_HZ: f64 = 4.0;
const RADAR_SECS: f64 = 4.0;
const RADAR_ALPHA: f32 = 0.35;
const RADAR_WIDTH: f32 = 0.12;
const LED_HZ: [f64; 3] = [0.5, 0.8, 1.3];
const LED_LOW: f32 = 0.4;

/// n7b: `t` held to the start of its idle step.
fn idle_step(t: f64) -> f64 {
    (t * IDLE_FPS).floor() / IDLE_FPS
}

/// n7b: the radar sweep's direction in turns `s` seconds after the popup
/// opened, from straight up.
fn radar_turn(s: f64) -> f32 {
    ((s / RADAR_SECS).rem_euclid(1.0) - 0.25) as f32
}

/// n7b: LED `i`'s alpha `s` seconds after the popup opened: on for the
/// first half of each period, at `LED_LOW` for the second.
fn led(i: usize, s: f64) -> f32 {
    if (s * LED_HZ[i]).rem_euclid(1.0) < 0.5 {
        1.0
    } else {
        LED_LOW
    }
}

/// The status mark: an orange ring with a dot, a spinner on a grey ring, a
/// check in a closing ring, or a grey X.
#[derive(Clone)]
struct Mark {
    mode: Mode,
    mode_at: f64,
    /// n7b: the radar sweep turns (open, working, not reduced), timed
    /// from when the popup opened.
    idle: bool,
    shown_at: f64,
}

impl Mark {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Done | Mode::Cancelled => self.mode_at + 0.4,
            _ => f64::INFINITY,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        let s = r.w.min(r.h) / 2.0 - 1.5;
        let k = ease_out((t - self.mode_at) / 0.3);
        let orange = hex(ORANGE);
        match self.mode {
            Mode::Loading | Mode::Listening => {
                p.stroke(&make::ring(c, s), &StrokeStyle::solid(1.6, orange));
                if self.idle {
                    let turn = radar_turn(idle_step(t) - self.shown_at);
                    p.shape(ShapeInstance::circle(
                        c,
                        s - 1.0,
                        Paint::conic(
                            c,
                            orange.fade(RADAR_ALPHA),
                            orange.fade(0.0),
                            turn,
                            RADAR_WIDTH,
                        ),
                    ));
                }
                let pulse = if reduced() {
                    1.0
                } else {
                    0.85 + 0.15 * (t * std::f64::consts::TAU).sin() as f32
                };
                p.shape(ShapeInstance::circle(
                    c,
                    s * 0.5 * pulse,
                    Paint::solid(orange),
                ));
            }
            Mode::Finishing => {
                p.stroke(&make::ring(c, s), &StrokeStyle::solid(2.6, hex(TRACK)));
                p.stroke(&make::spinner(c, s, t), &StrokeStyle::solid(2.6, orange));
            }
            Mode::Done => {
                p.stroke(&make::progress(c, s, k), &StrokeStyle::solid(1.6, orange));
                let check = Outline::new(&icons::check(), 0.05).transform(
                    &Affine::scale(s * 1.1 / 24.0)
                        .then(&Affine::translate(c.x - s * 0.55, c.y - s * 0.55)),
                );
                let len = check.length();
                p.outline(&check.trim(0.0, len * k), &StrokeStyle::solid(1.8, orange));
            }
            Mode::Cancelled => {
                let x = Outline::new(&icons::x(), 0.05).transform(
                    &Affine::scale(s * 1.2 / 24.0)
                        .then(&Affine::translate(c.x - s * 0.6, c.y - s * 0.6)),
                );
                let len = x.length();
                p.outline(&x.trim(0.0, len * k), &StrokeStyle::solid(1.6, hex(GREY)));
            }
        }
    }
}

impl Popup {
    pub(crate) fn mission_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.mission_panel().margin_edges(12.0, 0.0, 0.0, 12.0))
    }

    pub(crate) fn mission_panel(&self) -> Element<Msg> {
        let w = panel_w(&self.look);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let cancelled = mode == Mode::Cancelled;
        let accent = if cancelled { hex(GREY) } else { hex(ORANGE) };
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let inner = w - 2.0 * PAD;
        Element::panel("panel")
            .width(w)
            .pad_edges(PAD, PAD, PAD_BOTTOM, PAD)
            .gap(GAP)
            .radius(RADIUS)
            .fill(hex(SLATE).fade(0.96))
            .border(accent.fade(0.65), 1.5)
            .glow(accent.fade(0.12), 10.0, 0.5)
            .motion(Prop::Opacity, Spring::critical_to_98(0.2))
            .presence(Transition::fade().scale(0.98), exit)
            .child(self.mission_tiles(mode, inner))
            .child(self.mission_transcript(mode, inner, accent))
            .child(self.mission_wave(mode, mode_at, inner, accent))
            .child(self.mission_status(mode, mode_at, inner))
    }

    /// n7b: the idle motion runs: open, listening or loading, not reduced.
    fn mission_idle(&self, mode: Mode) -> bool {
        !self.closing
            && matches!(mode, Mode::Listening | Mode::Loading)
            && !reduced()
            && !self.look.on("reduced-motion")
    }

    /// The row of telemetry tiles.
    /// `s` cut with an ellipsis until it is at most `room` px wide, so no
    /// tile text crosses its tile's edge.
    fn fitted(&self, s: &str, style: &TextStyle, room: f32) -> String {
        let wide = |t: &str| self.measure.layout(t, style, None).size.w;
        if wide(s) <= room {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        (1..chars.len())
            .rev()
            .map(|n| format!("{}\u{2026}", chars[..n].iter().collect::<String>().trim_end()))
            .find(|t| wide(t) <= room)
            .unwrap_or_else(|| "\u{2026}".to_string())
    }

    fn mission_tiles(&self, mode: Mode, inner: f32) -> Element<Msg> {
        let f = fonts(&self.look.fonts);
        let k = (inner - 4.0 * TILE_GAP) / TILE_W.iter().sum::<f32>();
        let caps = TextStyle::new(f.caps, 9.5, 12.0).tracking(1.2);
        let value = TextStyle::new(f.bold, 13.5, 17.0);
        let small = TextStyle::new(f.ui, 9.5, 12.0);
        let roll = Spring::critical_to_98(0.3);
        let orange = hex(ORANGE);
        let facts = facts();
        let glyphs = tile_icons();
        let tile = |key: &'static str, n: usize, name: &'static str, glyph: Element<Msg>| {
            let tw = (TILE_W[n] * k).floor();
            Element::panel(key)
                .size(tw, TILES_H)
                .radius(4.0)
                .fill(hex(TILE).fade(0.7))
                .border(hex(TILE_EDGE), 1.0)
                .presence(
                    Transition::fade().offset(0.0, 4.0 + 2.0 * n as f32),
                    Transition::fade(),
                )
                .child(corners(
                    "ticks",
                    tw,
                    TILES_H,
                    Corners {
                        len: 5.0,
                        width: 1.4,
                        which: [true, false, true, false],
                        color: orange.fade(0.8),
                    },
                ))
                .child(at(
                    Element::label("name", name).text_style(caps).color(hex(LABEL)),
                    38.0,
                    15.0,
                ))
                .child(at(glyph, 12.0, 30.0))
        };
        let text = |key: &'static str, s: String, style: TextStyle, c: u32| {
            Element::label(key, s).text_style(style).color(hex(c))
        };

        // The model, or the engine when the config names none.
        let model = if facts.model.is_empty() {
            facts.engine.clone()
        } else {
            facts.model.clone()
        };
        let room = (TILE_W[0] * k).floor() - MODEL_X - TILE_RIGHT;
        let model = self.fitted(&model, &value, room);
        let about = self.fitted(&format!("{} \u{b7} Local", facts.engine), &small, room);
        let mut model_tile = tile(
            "model",
            0,
            "MODEL",
            icon("icon", glyphs.model.clone(), 20.0, orange),
        )
        .child(at(text("value", model, value, INK), MODEL_X, 30.0))
        .child(at(
            text("about", about, small, SOFT),
            38.0,
            50.0,
        ));
        if let Some(fraction) = self
            .kit_progress()
            .filter(|_| mode == Mode::Loading)
        {
            model_tile = model_tile.child(at(
                Element::progress(
                    "loading",
                    Progress {
                        fraction: Some(fraction),
                        track: hex(TRACK),
                        bar: orange,
                        since: self.shown_at,
                        period: 1.6,
                    },
                )
                .size((TILE_W[0] * k).floor() - 50.0, 2.0),
                38.0,
                64.0,
            ));
        }

        // GPU memory with a bar and the share in use.
        let vram_w = (TILE_W[1] * k).floor();
        let mut vram_tile = tile(
            "vram",
            1,
            "VRAM",
            icon("icon", glyphs.chip.clone(), 20.0, orange),
        );
        match vram() {
            Some((used, total)) => {
                let share = (used / total).clamp(0.0, 1.0);
                vram_tile = vram_tile
                    .child(at(
                        Element::row("value")
                            .align_items(AlignItems::END)
                            .child(digit_roll("used", &format!("{used:.1}"), value, hex(INK), roll))
                            .child(text("total", format!(" / {total:.1} GB"), small, LABEL)),
                        38.0,
                        30.0,
                    ))
                    .child(at(
                        Element::progress(
                            "bar",
                            Progress {
                                fraction: Some(share),
                                track: hex(TRACK),
                                bar: orange,
                                since: self.shown_at,
                                period: 1.6,
                            },
                        )
                        .size(vram_w - 88.0, 4.0),
                        38.0,
                        55.0,
                    ))
                    .child(at(
                        text("share", format!("{:.0}%", share * 100.0), small, LABEL),
                        vram_w - 40.0,
                        51.0,
                    ));
            }
            None => {
                vram_tile = vram_tile
                    .child(at(text("value", "n/a".into(), value, LABEL), 38.0, 30.0))
                    .child(at(text("about", "no GPU data".into(), small, SOFT), 38.0, 50.0));
            }
        }

        // Latency in ms, with a small signal meter that moves with the voice.
        let lat_w = (TILE_W[2] * k).floor();
        let ms = self
            .fam
            .latency
            .map_or("0".to_string(), |s| format!("{:.0}", s * 1000.0));
        let levels = self.kit_raw_levels();
        let live = mode == Mode::Listening;
        // n7b: while idle the meter reads the level 4 times a second.
        let idle = self.mission_idle(mode);
        let meter = Element::canvas(
            "meter",
            if live { f64::INFINITY } else { 0.0 },
            move |t, r, p| {
                let at = if idle {
                    (t * TELEMETRY_HZ).floor() / TELEMETRY_HZ
                } else {
                    t
                };
                let level = if live { (levels.at(at) * 1.6).min(1.0) } else { 0.0 };
                for i in 0..5 {
                    let h = r.h * (0.3 + 0.175 * i as f32);
                    let on = (i as f32) < 1.0 + level * 4.0;
                    let x = r.x + i as f32 * (r.w / 5.0);
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(x, r.y + r.h - h, 2.4, h), 0.6),
                        Paint::solid(if on { hex(ORANGE) } else { hex(TRACK) }),
                    ));
                }
            },
        )
        .size(20.0, 10.0);
        let latency_tile = tile(
            "latency",
            2,
            "LATENCY",
            icon("icon", glyphs.timer.clone(), 20.0, orange),
        )
        .child(at(
            Element::row("value")
                .align_items(AlignItems::END)
                .child(digit_roll("ms", &ms, value, hex(INK), roll))
                .child(text("unit", " ms".into(), small, LABEL)),
            38.0,
            30.0,
        ))
        .child(at(text("about", "(avg)".into(), small, SOFT), 38.0, 50.0))
        .child(at(meter, lat_w - 32.0, 52.0));

        // The microphone, cut to fit.
        let mic_w = (TILE_W[3] * k).floor();
        let mut mic = facts.mic.clone();
        let fits = ((mic_w - 46.0) / 7.5) as usize;
        if mic.chars().count() > fits {
            mic = mic.chars().take(fits.saturating_sub(1)).collect::<String>() + "\u{2026}";
        }
        let mic_tile = tile(
            "mic",
            3,
            "MICROPHONE",
            icon("icon", glyphs.mic.clone(), 20.0, orange),
        )
        .child(at(text("value", mic, value, INK), 38.0, 30.0))
        .child(at(text("about", "Input".into(), small, SOFT), 38.0, 50.0));

        // Elapsed time, with three dots that run once a second; while idle
        // they blink as status LEDs instead (n7b).
        let clock = format!("00:{}", self.clock);
        let shown_at = self.shown_at;
        let dots = Element::canvas(
            "dots",
            if live { f64::INFINITY } else { 0.0 },
            move |t, r, p| {
                let u = if live && !reduced() { (t.fract() * 3.0) as usize } else { 3 };
                for i in 0..3 {
                    let c = Point::new(r.x + 2.0 + i as f32 * 6.0, r.y + r.h / 2.0);
                    let a = if idle {
                        led(i, idle_step(t) - shown_at)
                    } else if i == u {
                        1.0
                    } else {
                        0.45
                    };
                    p.shape(ShapeInstance::circle(c, 1.4, Paint::solid(hex(ORANGE).fade(a))));
                }
            },
        )
        .size(18.0, 6.0);
        let elapsed_tile = tile(
            "elapsed",
            4,
            "ELAPSED",
            icon("icon", glyphs.timer.clone(), 20.0, orange),
        )
        .child(at(digit_roll("clock", &clock, value, hex(INK), roll), 38.0, 30.0))
        .child(at(dots, 40.0, 53.0));

        Element::row("tiles")
            .height(TILES_H)
            .gap(TILE_GAP)
            .child(model_tile)
            .child(vram_tile)
            .child(latency_tile)
            .child(mic_tile)
            .child(elapsed_tile)
    }

    /// The big tile with the words.
    fn mission_transcript(&self, mode: Mode, inner: f32, accent: Color) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let h = transcript_h(self.lines.min(MAX_LINES)) + lattice_room(l);
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown[..self.edit_room()])
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(INK))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0)));
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(text_width(l)),
            hex(INK),
            accent,
        );
        let live = mode == Mode::Listening;
        let name = match mode {
            Mode::Listening | Mode::Loading => "LIVE TRANSCRIPT",
            _ => "TRANSCRIPT",
        };
        let pulse = Element::canvas(
            "pulse",
            if live { f64::INFINITY } else { 0.0 },
            move |t, r, p| {
                let a = if live && !reduced() {
                    0.6 + 0.4 * (t * std::f64::consts::TAU).cos() as f32
                } else {
                    1.0
                };
                p.shape(ShapeInstance::circle(
                    Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0),
                    r.w / 2.0,
                    Paint::solid(accent.fade(a)),
                ));
            },
        )
        .size(7.0, 7.0);
        // A small waveform glyph at the top right.
        let glyph = Element::canvas("glyph", 0.0, move |_, r, p| {
            for (i, hk) in [0.35, 0.7, 1.0, 0.6, 0.3].iter().enumerate() {
                let bh = r.h * hk;
                let x = r.x + i as f32 * r.w / 5.0;
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x, r.y + (r.h - bh) / 2.0, 2.0, bh), 1.0),
                    Paint::solid(accent),
                ));
            }
        })
        .size(17.0, 17.0);
        // The word count and the pace at the tile's top right: each new word
        // casts a block into the notch by the count (MO-65), and the pace
        // digits lean with the speech rate (MO-66).
        // Both are off by default on mission, so the shaping and the casts
        // they need run only when one is on: on room tone the view is rebuilt
        // every frame, and shaping is most of its cost.
        let count_cast = self.motion("count-cast");
        let lean_pace = self.motion("lean-pace");
        let digits = TextStyle::new(f.bold, 12.0, 14.0);
        let caps = TextStyle::new(f.caps, 9.0, 12.0).tracking(1.3);
        let stat = |key: &'static str, n: Element<Msg>, unit: &'static str| {
            Element::row(key)
                .align_items(AlignItems::END)
                .child(n)
                .child(
                    Element::label("unit", unit)
                        .text_style(caps)
                        .color(hex(LABEL)),
                )
        };
        let roll = Spring::critical_to_98(0.28);
        let mut tile = Element::panel("transcript")
            .size(inner, h)
            .radius(5.0)
            .fill(hex(TILE).fade(0.55))
            .border(accent.fade(0.35), 1.0);
        if count_cast || lean_pace {
            let count = self.tally_count();
            let cell = self.measure.layout("0", &digits, None).size.w;
            let width = |n: usize, unit: &str| {
                cell * n as f32 + self.measure.layout(unit, &caps, None).size.w
            };
            let count_x = inner - 46.0 - width(count.len(), " WORDS");
            if count_cast {
                let notch = Rect::new(count_x - NOTCH_ROOM + 2.0, LABEL_Y - 1.0, 2.5, 13.0);
                let casts = self.tally_casts(Point::new(TEXT_X, TEXT_TOP), notch, accent, false);
                tile = tile
                    .child(at(
                        Element::canvas("cast", casts.until(), move |t, r, p| {
                            casts.draw(t, r, p)
                        })
                        .size(inner, h),
                        0.0,
                        0.0,
                    ))
                    .child(at(
                        stat(
                            "count",
                            digit_roll("n", &count, digits, hex(INK), roll),
                            " WORDS",
                        ),
                        count_x,
                        LABEL_Y - 2.0,
                    ));
            }
            if lean_pace {
                let (wpm, lean) = self.tally_pace();
                let wpm = wpm.to_string();
                let pace_x = count_x - NOTCH_ROOM - 14.0 - width(wpm.len(), " WPM");
                tile = tile.child(at(
                    stat(
                        "pace",
                        tally::lean_digits("n", &wpm, digits, hex(INK), cell, lean),
                        " WPM",
                    ),
                    pace_x,
                    LABEL_Y - 2.0,
                ));
            }
        }
        // LT-07: the kinetic lattice in blueprint style under the words.
        if l.on(wild_space::KINETIC_LATTICE) {
            let set = wild_space::LatticeSet {
                preset: LatticeStyle::Blueprint,
                color: hex(LABEL).fade(0.55),
                accent,
                scale: 1.0,
                label: Some(TextStyle::new(f.caps, 9.0, 11.0).tracking(0.6)),
                gutter: LATTICE_GUTTER,
                opacity: dim,
            };
            let style = l.text_style();
            tile = tile.children(
                self.lattice_lines("mission", &self.shown[..self.edit_room()], &style, text_width(l), &set)
                    .map(|e| at(e, TEXT_X, TEXT_TOP)),
            );
        }
        // MO-35: the hairline shapes the whole transcript to find its line,
        // so it is built only once the limit is near; until then it would
        // draw nothing. The audio frames rebuild the view while recording.
        let limit_near = self.limit_secs().is_some_and(|limit| {
            self.extra.now + limit::WINDOW_SECS + 1.0 >= self.shown_at + limit
        });
        let hairline = if limit_near {
            self.limit_line(
                Point::new(TEXT_X, TEXT_TOP),
                Size::new(inner, h),
                hex(INK).fade(0.5),
                hex(0xFF3B30),
            )
        } else {
            None
        };
        tile.children(hairline)
        .child(corners(
                "brackets",
                inner,
                h,
                Corners {
                    len: 22.0,
                    width: 1.8,
                    which: [true, true, true, true],
                    color: accent,
                },
            ))
            .child(at(pulse, TEXT_X - 1.0, LABEL_Y + 2.0))
            .child(at(
                Element::label(format!("label-{name}"), name)
                    .text_style(TextStyle::new(f.caps, 10.0, 12.0).tracking(1.4))
                    .color(hex(LABEL))
                    .presence(Transition::fade().wipe(1.0), Transition::fade()),
                TEXT_X + 15.0,
                LABEL_Y,
            ))
            .child(at(glyph, inner - 34.0, LABEL_Y - 3.0))
            .child(at(
                self.signal_marks(
                    Element::column("text")
                        .width(text_width(l))
                        .child(words),
                    &self.shown[..self.edit_room()],
                    text_width(l),
                    (0.0, 0.0),
                    hex(INK),
                ),
                TEXT_X,
                TEXT_TOP,
            ))
            // MO-71: the sound under the newest words.
            .children(self.receipts_element(
                "mission",
                &self.shown,
                &l.text_style(),
                TEXT_X,
                TEXT_TOP,
                text_width(l),
                accent,
            ))
    }

    /// The waveform strip and the LIVE meter.
    fn mission_wave(&self, mode: Mode, mode_at: f64, inner: f32, accent: Color) -> Element<Msg> {
        let f = fonts(&self.look.fonts);
        let wave = Wave {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
            progress: self.kit_progress(),
            window: self.look.window,
        };
        let until = wave.until();
        let label = match mode {
            Mode::Done => "SENT",
            Mode::Cancelled => "OFF",
            _ => "LIVE",
        };
        Element::panel("wave")
            .size(inner, WAVE_H)
            .radius(5.0)
            .fill(hex(TILE).fade(0.55))
            .border(hex(TILE_EDGE), 1.0)
            .child(corners(
                "brackets",
                inner,
                WAVE_H,
                Corners {
                    len: 9.0,
                    width: 1.4,
                    which: [true, false, false, true],
                    color: accent.fade(0.8),
                },
            ))
            .child(at(
                Element::canvas("bars", until, move |t, r, p| wave.draw(t, r, p))
                    .size(inner, WAVE_H),
                0.0,
                0.0,
            ))
            .child(at(
                Element::label(format!("meter-{label}"), label)
                    .text_style(TextStyle::new(f.caps, 9.0, 11.0).tracking(1.3))
                    .color(hex(LABEL))
                    .presence(Transition::fade(), Transition::fade()),
                inner - METER_W + 15.0,
                WAVE_H - 20.0,
            ))
    }

    /// The status bar: the mark, the state and what to do, a progress line
    /// while the final text is made, and the name at the right.
    fn mission_status(&self, mode: Mode, mode_at: f64, inner: f32) -> Element<Msg> {
        let f = fonts(&self.look.fonts);
        let big = TextStyle::new(f.bold, 13.0, 17.0);
        let small = TextStyle::new(f.ui, 11.0, 15.0);
        let status = self.status();
        let mark = Mark {
            mode,
            mode_at,
            idle: self.mission_idle(mode),
            shown_at: self.shown_at,
        };
        let until = mark.until();
        let state = match mode {
            Mode::Done => self.delivery_word().to_string(),
            Mode::Cancelled => "Cancelled".to_string(),
            _ => status.state.clone(),
        };
        let detail = match mode {
            Mode::Done => self
                .kit_target()
                .map(|a| format!("into {a}"))
                .unwrap_or_default(),
            Mode::Cancelled => "nothing was typed".to_string(),
            _ => status
                .detail
                .trim_start_matches([' ', '\u{b7}'])
                .trim()
                .to_string(),
        };
        // One state label and one detail label whose text changes in place:
        // keyed by state, the old pair faded out where the new pair faded in,
        // and the two texts crossed ("Loading" over "Listening").
        let mut words = Element::row("words")
            .align_items(AlignItems::CENTER)
            .child(Element::label("state", state.as_str()).text_style(big).color(hex(INK)));
        if !detail.is_empty() {
            words = words.child(
                Element::label("detail", format!("  \u{b7}  {detail}"))
                    .text_style(small)
                    .color(hex(LABEL))
                    // Its x follows the state word's width: a slide from
                    // the old width put the dot against the new word.
                    .motion(Prop::Position, Motion::Snap),
            );
        }
        let mut left = Element::column("left").gap(5.0).child(words);
        // MO-41: the seconds of audio left as a ruler, in place of the
        // progress line, once the daemon reports them.
        if let Some(ruler) = self.audio_ruler(hex(ORANGE), hex(TRACK)) {
            left = left.child(
                ruler
                    .width(inner - 230.0)
                    .presence(Transition::fade(), Transition::fade()),
            );
        } else if let Some(fraction) = self
            .kit_progress()
            .filter(|_| mode == Mode::Finishing)
        {
            left = left.child(
                Element::progress(
                    "progress",
                    Progress {
                        fraction: Some(fraction),
                        track: hex(TRACK),
                        bar: hex(ORANGE),
                        since: self.shown_at,
                        period: 1.6,
                    },
                )
                .size(inner - 230.0, 3.0)
                .presence(Transition::fade(), Transition::fade()),
            );
        }
        let caps = TextStyle::new(f.caps, 8.5, 10.5).tracking(1.4);
        let slash = Element::canvas("slash", 0.0, |_, r, p| {
            for i in 0..2 {
                let x = r.x + i as f32 * 4.0;
                p.shape(ShapeInstance::line(
                    Point::new(x, r.y + r.h),
                    Point::new(x + 7.0, r.y),
                    1.0,
                    Paint::solid(hex(SOFT)),
                ));
            }
        })
        .size(12.0, 12.0);
        let name = Element::row("name")
            .align_items(AlignItems::CENTER)
            .gap(9.0)
            .child(slash)
            .child(
                Element::column("lines")
                    .gap(2.0)
                    .child(Element::label("brand", "VOXTYPE").text_style(caps).color(hex(LABEL)))
                    .child(
                        Element::label("what", "VOICE DICTATION")
                            .text_style(caps)
                            .color(hex(SOFT)),
                    ),
            );
        Element::panel("status")
            .direction_row()
            .size(inner, STATUS_H)
            .align_items(AlignItems::CENTER)
            .pad_edges(0.0, 14.0, 0.0, 9.0)
            .radius(5.0)
            .fill(hex(TILE).fade(0.55))
            .border(hex(TILE_EDGE), 1.0)
            .child(
                Element::canvas("mark", until, move |t, r, p| mark.draw(t, r, p))
                    .size(26.0, 26.0),
            )
            .child(left.margin_edges(0.0, 0.0, 0.0, 14.0))
            .child(Element::row("spacer").grow(1.0))
            .child(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_config_keeps_the_designed_width() {
        let design = (MIN_WIDTH * WIDEN).round();
        let cfg = OsdConfig { width_px: 400, ..OsdConfig::default() };
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        assert_eq!(panel_w(&look), design);
        look.width = 600.0;
        assert_eq!(panel_w(&look), design);
        look.width = 800.0;
        assert!(panel_w(&look) > design);
    }

    #[test]
    fn long_model_name_is_cut_to_its_tile() {
        let cfg = OsdConfig::default();
        let plasma = PlasmaTheme::default();
        let reg = load_fonts(&plasma).unwrap();
        let look = Look::new(&cfg, &plasma, String::new(), &reg);
        let popup = Popup::new(look, &reg);
        let style = TextStyle::new(fonts(&popup.look.fonts).bold, 13.5, 17.0);
        let room = 120.0;
        let w = |t: &str| popup.measure.layout(t, &style, None).size.w;
        let long = popup.fitted("cohere-transcribe-q4f1-long", &style, room);
        assert!(long.ends_with('\u{2026}') && w(&long) <= room, "{long}");
        assert_eq!(popup.fitted("Cohere", &style, room), "Cohere");
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use voxtype::osd::config::OsdTheme;

    /// M7.L3: Mission edits in its own layout, with its hinge and revision
    /// address off (quiet list). With the caret on the newest, a middle and
    /// the top line, at both widths: the caret on its glyph within 1 px in
    /// every frame, the popup's size unchanged, and the golden
    /// `tests/fixtures/osd-vo/edit-mission.txt`.
    #[test]
    fn mission_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Mission, "mission");
    }

    /// M7.L3: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn mission_edits_within_the_envelope() {
        envelope(OsdTheme::Mission, "mission");
    }

    /// Every window word draws while editing, also just after the panel
    /// grew a line (no `room()` hold-back).
    #[test]
    fn mission_draws_every_word_while_editing() {
        super::super::type_look::edit_tests::every_word_draws(OsdTheme::Mission);
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn the_radar_turns_once_every_4_s_from_straight_up() {
        assert_eq!(radar_turn(0.0), -0.25);
        assert!((radar_turn(1.0) - 0.0).abs() < 1e-6);
        assert!((radar_turn(2.6) - radar_turn(2.6 + RADAR_SECS)).abs() < 1e-6);
    }

    #[test]
    fn the_leds_blink_at_their_own_rates_between_40_and_100_percent() {
        for (i, hz) in LED_HZ.iter().enumerate() {
            let period = 1.0 / hz;
            assert_eq!(led(i, 0.1 * period), 1.0);
            assert_eq!(led(i, 0.6 * period), LED_LOW);
            assert_eq!(led(i, 1.1 * period), 1.0);
        }
    }

    // The LEDs dim to 40 %, and the radar sweep stays soft.
    const _: () = assert!(LED_LOW == 0.4 && RADAR_ALPHA <= 0.35);
}
