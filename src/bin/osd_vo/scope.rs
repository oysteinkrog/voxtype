//! The `scope` look of the vo popup, after mock-up 18: a green oscilloscope
//! instrument. A bright vector trace runs over a graticule with a dBFS
//! scale, a readout column shows the input level, the elapsed time and the
//! words per minute with small segment meters, and the words show in a
//! monospaced readout panel with corner brackets and a block caret.
//!
//! The trace is rebuilt from the audio levels every frame as one polyline
//! with a glow; it freezes at the stop while a cursor sweeps with the
//! progress, and goes flat once the text is pasted. The readout numbers
//! change at most twice a second.
//!
//! The mock-up's latency and model readouts have no source in the popup, so
//! those two rows show the elapsed time and the word count instead.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).
//!
//! While the popup is open it stays a little alive (m18, MOTION-PLAN rule 8):
//! the trace keeps a floor of at most 1 px from the real room level, the
//! graticule's glow breathes from 6 % to 9 % over 4 s, and the trigger
//! marker blinks at 1 Hz. The glow and the blink are ambient loops stepped
//! at 15 a second; all three stop under reduced motion and leave the view
//! as the popup closes. None of them is near the transcript.

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, Contour, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo::scene::{Loop, LoopProp};

const BG: u32 = 0x04100b;
const PANEL: u32 = 0x061711;
const EDGE: u32 = 0x1fcf7c;
const FRAME: u32 = 0x16573a;
const GRID: u32 = 0x0f3a27;
const TRACE: u32 = 0x3dff8f;
const VALUE: u32 = 0x5cffa4;
const LABEL: u32 = 0x7fd6a6;
const TEXT: u32 = 0x5ef7a3;
const DIM: u32 = 0x2d7a54;
const OFF: u32 = 0x10392a;

fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

/// The mock-up's 912 x 598 panel at 0.75.
const S: f32 = 0.75;
const W: f32 = 912.0 * S;
const H: f32 = 598.0 * S;
const MARGIN: f32 = 14.0;

/// Boxes, in the mock-up's pixels before scaling.
const SCOPE: (f32, f32, f32, f32) = (13.0, 58.0, 723.0, 230.0);
const SCALE_W: f32 = 60.0;
const READ: (f32, f32, f32, f32) = (745.0, 58.0, 155.0, 230.0);
const TRANS: (f32, f32, f32, f32) = (13.0, 300.0, 887.0, 255.0);
const INNER: (f32, f32, f32, f32) = (20.0, 336.0, 873.0, 210.0);
const STATUS_Y: f32 = 573.0;

const TEXT_PX: f32 = 28.0 * S;
const LINE: f32 = 43.0 * S;
const TEXT_X: f32 = 50.0 * S;
const TEXT_TOP: f32 = 350.0 * S;
const WRAP: f32 = 800.0 * S;
pub(crate) const MAX_LINES: u32 = 4;

/// Seconds of audio across the graticule.
const WINDOW: f64 = 2.6;
/// Sample points per second of the trace's carrier.
const CARRIER_HZ: f64 = 420.0;
/// The level that reaches the top of the graticule.
const FULL: f32 = 0.95;

/// With `sentence-trace` (MO-63): each word keeps the stretch of trace it
/// came from. Once the word arrives, that stretch freezes, shrinks and
/// drops into a strip along the foot of the graticule with a tick at its
/// start, so the strip reads as the sentence so far.
pub(crate) const SENTENCE_TRACE: &str = "sentence-trace";
/// The strip's height, in the mock-up's pixels before scaling.
const STRIP_H: f32 = 38.0;
/// Strip pixels per second of sound, and the narrowest word.
const STRIP_PX_PER_SEC: f32 = 90.0 * S;
const STRIP_MIN_W: f32 = 6.0;
const STRIP_GAP: f32 = 3.0;
/// How long a stretch takes to drop into the strip, and how long its tick
/// stays bright after it lands.
const STRIP_FLY: f64 = 0.5;
const STRIP_TICK: f64 = 0.6;
/// Envelope points kept per word, and words kept.
const STRIP_POINTS: usize = 32;
const STRIP_WORDS: usize = 64;

/// LT-05: the lattice's pause labels under the newest line, in pixels at 1x:
/// big enough to read, small enough to sit between the descenders and the
/// transcript frame when the newest line is the fourth.
const LATTICE_LABEL_PX: f32 = 10.0;

/// m18: the most the trace's live floor swings, in logical pixels, reached
/// at the room-tone level (`ROOM_TONE_DBFS`); true silence draws it flat.
const FLOOR_PX: f32 = 1.0;
/// m18: the graticule glow's alpha at rest, its swing on top, and the
/// seconds per breath.
const GLOW_ALPHA: f32 = 0.06;
const GLOW_SWING: f64 = 0.03;
const GLOW_PERIOD: f64 = 4.0;
/// m18: the trigger marker's dim opacity in the off half of its 1 s blink.
const TRIGGER_DIM: f32 = 0.35;
/// m18: the idle loops' pace, the slow-loop cap of MOTION-PLAN rule 8.
const IDLE_FPS: f64 = 15.0;

/// MO-63: one word's stretch of trace, frozen when the word came.
#[derive(Clone, Debug, PartialEq)]
struct Frozen {
    id: u32,
    /// The sound it came from, in scene time.
    sound: (f64, f64),
    /// When it froze.
    at: f64,
    /// The envelope over the sound, 0 to 1, `STRIP_POINTS` long.
    env: Arc<[f32]>,
}

/// The envelope of `levels` (newest last, the newest at `newest_at`) over
/// the sound from `a` to `b`, as `n` points scaled like the trace.
fn envelope(levels: &[f32], newest_at: f64, a: f64, b: f64, n: usize) -> Vec<f32> {
    let len = levels.len();
    (0..n)
        .map(|i| {
            let s = a + (b - a) * i as f64 / (n - 1).max(1) as f64;
            let back = ((newest_at - s) * FRAME_HZ as f64).max(0.0);
            let j = (len as f64 - 1.0 - back).round();
            if len == 0 || j < 0.0 {
                return 0.0;
            }
            (levels[j as usize] / FULL).min(1.0).powf(1.7)
        })
        .collect()
}

/// The right edge of each slot in the strip, oldest first. Each word makes
/// room as it lands: `k` is how far each has landed, 0 to 1, and a word
/// moves left by the width of each newer word times that word's `k`.
fn slot_rights(widths: &[f32], k: &[f32], right: f32) -> Vec<f32> {
    let mut out = vec![0.0; widths.len()];
    let mut x = right;
    for i in (0..widths.len()).rev() {
        out[i] = x;
        x -= (widths[i] + STRIP_GAP) * k[i];
    }
    out
}

fn strip_width(sound: (f64, f64)) -> f32 {
    ((sound.1 - sound.0) as f32 * STRIP_PX_PER_SEC).max(STRIP_MIN_W)
}

#[derive(Clone, Copy)]
struct Fonts {
    mono: u32,
    bold: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            mono: add("JetBrains Mono:medium", MONO),
            bold: add("JetBrains Mono:semibold", MONO),
        }
    })
}

/// MO-50 with the lattice: a cancel takes the words back into the caret, so
/// the strands under them fade out over the rewind instead of staying behind
/// on an empty line. Reduced motion shows the end state at once.
fn lattice_rewind(e: Element<Msg>, rewinding: bool, reduced: bool) -> Element<Msg> {
    match (rewinding, reduced) {
        (false, _) => e,
        (true, true) => e.opacity(0.0).motion(Prop::Opacity, Motion::Snap),
        (true, false) => e.opacity(0.0).motion(Prop::Opacity, Spring::critical_to_98(0.3)),
    }
}

fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).mono, TEXT_PX, LINE).tracking(0.4)
}

pub(crate) fn text_width(_look: &Look) -> f32 {
    WRAP
}

fn at<M>(e: Element<M>, x: f32, y: f32) -> Element<M> {
    e.layout(|s| {
        s.position = taffy::Position::Absolute;
        s.inset = taffy::Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        };
    })
}

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: i64) -> f32 {
    let mut x = (n as u64 as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn scaled(r: (f32, f32, f32, f32)) -> Rect {
    Rect::new(r.0 * S, r.1 * S, r.2 * S, r.3 * S)
}

/// m18: the trace's live floor for `level`, in pixels: up to `FLOOR_PX`,
/// reached at `room` (the room-tone level), so the quiet trace still shows
/// the room it hears. Flat at true silence and with `room` 0 (off).
fn live_floor(level: f32, room: f32) -> f32 {
    if room <= 0.0 {
        return 0.0;
    }
    FLOOR_PX * (level / room).clamp(0.0, 1.0).sqrt()
}

/// The graticule's box in the scene's frame, without the card margin.
fn graticule() -> Rect {
    let sc = scaled(SCOPE);
    Rect::new(
        sc.x + SCALE_W * S + 1.0,
        sc.y + 1.0,
        sc.w - SCALE_W * S - 14.0 * S,
        sc.h - 14.0 * S,
    )
}

/// The graticule's lines, the strip's foot cut off when `strip` is on.
fn grid_lines(strip: bool) -> Vec<Rect> {
    let g = graticule();
    let (cols, rows) = (24, 8);
    let foot = if strip { g.h - STRIP_H * S } else { g.h };
    let mut out = Vec::with_capacity(cols + rows + 2);
    for i in 0..=cols {
        out.push(Rect::new(g.x + g.w * i as f32 / cols as f32, g.y, 1.0, foot));
    }
    for j in 0..=rows {
        let y = g.y + g.h * j as f32 / rows as f32;
        if y <= g.y + foot {
            out.push(Rect::new(g.x, y, g.w, 1.0));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Loading,
    Listening,
    Finishing,
    Done,
    Cancelled,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Loading => "LOADING",
            Mode::Listening => "LISTENING",
            Mode::Finishing => "FINISHING",
            Mode::Done => "DONE",
            Mode::Cancelled => "CANCELLED",
        }
    }
}

/// State only the scope look keeps.
pub struct State {
    mode: Option<Mode>,
    mode_at: f64,
    prog: (f32, f32, f64),
    /// The readouts as last shown, and when they last changed.
    dbfs: Option<f32>,
    wpm: u32,
    read_at: f64,
    word_at: f64,
    words: usize,
    /// MO-63: the words' frozen stretches of trace, in word order.
    sentence: Vec<Frozen>,
}

impl Default for State {
    fn default() -> State {
        State {
            mode: None,
            mode_at: f64::NEG_INFINITY,
            prog: (0.0, 0.0, 0.0),
            dbfs: None,
            wpm: 0,
            read_at: f64::NEG_INFINITY,
            word_at: f64::NEG_INFINITY,
            words: 0,
            sentence: Vec::new(),
        }
    }
}

/// What the canvas draws.
#[derive(Clone)]
struct Scene {
    mode: Mode,
    mode_at: f64,
    shown_at: f64,
    levels: Arc<[f32]>,
    newest_at: f64,
    /// The trace's clock during a rewind cancel (MO-50).
    scrub: endings::Scrub,
    finish_at: Option<f64>,
    final_at: Option<f64>,
    prog: Option<(f32, f32, f64)>,
    /// Lit segments of the three meters, out of 14, 10 and 10.
    meters: [usize; 3],
    caret: Option<(Point, f64)>,
    /// Onsets for the caret's foot (MO-73), when the look has it on.
    joint: Option<Arc<[f64]>>,
    /// MO-63: the frozen words, and whether motion is reduced.
    sentence: Option<(Arc<[Frozen]>, bool)>,
    /// m18: the trace level of room tone, where the live floor reaches
    /// `FLOOR_PX`, and whether the idle motion holds still (reduced motion):
    /// the trace then draws room tone flat.
    room: f32,
    still: bool,
}

impl Scene {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing => now + 0.25,
            _ => (now + 0.2).max(self.mode_at + 0.6),
        }
    }

    fn level_at(&self, s: f64) -> f32 {
        let n = self.levels.len();
        if n == 0 {
            return 0.0;
        }
        let back = ((self.newest_at - s) * FRAME_HZ as f64).max(0.0);
        let i = (n - 1) as f64 - back;
        if i < 0.0 {
            return 0.0;
        }
        let (a, k) = (i.floor() as usize, (i - i.floor()) as f32);
        let b = (a + 1).min(n - 1);
        self.levels[a] * (1.0 - k) + self.levels[b] * k
    }

    fn draw(&self, t: f64, _rect: Rect, p: &mut Painter) {
        let card = RRect::new(Rect::new(MARGIN, MARGIN, W, H), 12.0 * S);
        p.shape(ShapeInstance::rrect(
            card,
            Paint::glow(hex(EDGE).fade(0.25), 6.0, 0.5),
        ));
        p.shape(
            ShapeInstance::rrect(card, Paint::solid(hex(BG).fade(0.96)))
                .with_stroke(Paint::solid(hex(EDGE).fade(0.85)), 1.2),
        );
        p.layer(Affine::translate(MARGIN, MARGIN), 1.0, |p| {
            self.draw_frames(p);
            self.draw_grid(p);
            self.draw_trace(t, p);
            self.draw_sentence(t, p);
            self.draw_meters(p);
            self.draw_icons(t, p);
            self.draw_caret(t, p);
        });
    }

    fn boxed(&self, r: Rect, radius: f32, p: &mut Painter) {
        p.shape(
            ShapeInstance::rrect(RRect::new(r, radius), Paint::solid(hex(PANEL).fade(0.9)))
                .with_stroke(Paint::solid(hex(FRAME)), 1.0),
        );
    }

    fn draw_frames(&self, p: &mut Painter) {
        // The header rule.
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(0.0, 46.0 * S, W, 1.0), 0.0),
            Paint::solid(hex(FRAME)),
        ));
        self.boxed(scaled(SCOPE), 6.0 * S, p);
        let sc = scaled(SCOPE);
        self.boxed(Rect::new(sc.x, sc.y, SCALE_W * S, sc.h), 6.0 * S, p);
        self.boxed(scaled(READ), 6.0 * S, p);
        self.boxed(scaled(TRANS), 8.0 * S, p);
        let inner = scaled(INNER);
        p.shape(
            ShapeInstance::rrect(RRect::new(inner, 2.0), Paint::solid(hex(BG).fade(0.6)))
                .with_stroke(Paint::solid(hex(FRAME).fade(0.8)), 1.0),
        );
        // Corner brackets in the transcript box.
        for c in vo::scene::path::make::brackets(
            inner.x + 7.0 * S,
            inner.y + 7.0 * S,
            inner.w - 14.0 * S,
            inner.h - 14.0 * S,
            10.0 * S,
        ) {
            p.stroke(&c, &StrokeStyle::solid(1.4, hex(TEXT)));
        }
        // The EN tag.
        let tag = Rect::new(862.0 * S, 307.0 * S, 30.0 * S, 22.0 * S);
        p.shape(
            ShapeInstance::rrect(RRect::new(tag, 3.0), Paint::NONE)
                .with_stroke(Paint::solid(hex(EDGE).fade(0.8)), 1.0),
        );
    }

    fn graticule(&self) -> Rect {
        graticule()
    }

    fn draw_grid(&self, p: &mut Painter) {
        let g = self.graticule();
        let (cols, rows) = (24, 8);
        for i in 0..=cols {
            let x = g.x + g.w * i as f32 / cols as f32;
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x, g.y, 1.0, g.h), 0.0),
                Paint::solid(hex(GRID)),
            ));
        }
        for j in 0..=rows {
            let y = g.y + g.h * j as f32 / rows as f32;
            let c = if j == rows / 2 { hex(GRID).fade(1.0) } else { hex(GRID) };
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(g.x, y, g.w, 1.0), 0.0),
                Paint::solid(c),
            ));
        }
    }

    /// The trace: a dense carrier under the voice's envelope, newest at the
    /// right. Each point's sign and size come from its own sample number,
    /// so the trace scrolls rather than flickers.
    fn draw_trace(&self, t: f64, p: &mut Painter) {
        let g = self.graticule();
        let cy = g.y + g.h / 2.0;
        let t_eff = self.scrub.map(self.finish_at.map_or(t, |f| t.min(f)));
        let flat = self.final_at.map_or(0.0, |f| ease_out((t - f) / 0.35));
        let frozen = self.finish_at.is_some();
        let n = (g.w / 1.25) as usize;
        let mut pts = Vec::with_capacity(n);
        for i in 0..n {
            let u = i as f64 / (n - 1) as f64;
            let s = t_eff - (1.0 - u) * WINDOW;
            let k = (s * CARRIER_HZ).floor() as i64;
            // A spiky envelope: loud syllables stand out of a low floor.
            let level = self.level_at(s);
            let env = if self.still && level <= self.room { 0.0 } else { (level / FULL).min(1.0).powf(1.7) };
            let floor = if self.still { 0.0 } else { live_floor(level, self.room) };
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            let z = noise(k);
            let mag = 0.08 + 0.92 * z * z;
            let swing = (env * (g.h / 2.0 - 4.0)).max(floor);
            let y = cy - sign * mag * swing * (1.0 - flat);
            pts.push(Point::new(g.x + u as f32 * g.w, y));
        }
        let colour = if self.mode == Mode::Cancelled {
            hex(DIM)
        } else if frozen {
            hex(TRACE).fade(0.55)
        } else {
            hex(TRACE)
        };
        let line = Contour::polyline(&pts, false);
        p.stroke(&line, &StrokeStyle::solid(2.5, colour.fade(0.14)).glow(4.0));
        p.stroke(&line, &StrokeStyle::solid(1.1, colour));
        // While finishing, a cursor sweeps the frozen trace with the progress.
        if let (Mode::Finishing, Some((a, b, at))) = (self.mode, self.prog) {
            let f = a + (b - a) * ease_out((t - at) / 0.35);
            let x = g.x + g.w * f;
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x - 0.75, g.y, 1.5, g.h), 0.0),
                Paint::glow(hex(TRACE).fade(0.7), 4.0, 0.7),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x - 0.75, g.y, 1.5, g.h), 0.0),
                Paint::solid(hex(TRACE)),
            ));
        }
    }

    /// MO-63: the strip of frozen words along the foot of the graticule.
    /// A word that just came drops from where its sound is on the trace;
    /// with reduced motion it shows in its place at once.
    fn draw_sentence(&self, t: f64, p: &mut Painter) {
        let Some((words, reduced)) = &self.sentence else {
            return;
        };
        let g = self.graticule();
        let h = STRIP_H * S;
        let y0 = g.y + g.h - h;
        p.shape(
            ShapeInstance::rrect(RRect::new(Rect::new(g.x, y0, g.w, h), 0.0), Paint::solid(hex(BG).fade(0.92)))
                .with_stroke(Paint::solid(hex(GRID).fade(1.0)), 1.0),
        );
        if words.is_empty() {
            return;
        }
        let landed = |f: &Frozen| if *reduced { 1.0 } else { ease_out((t - f.at) / STRIP_FLY) };
        let widths: Vec<f32> = words.iter().map(|f| strip_width(f.sound)).collect();
        let k: Vec<f32> = words.iter().map(landed).collect();
        let rights = slot_rights(&widths, &k, g.x + g.w - 4.0);
        let t_eff = self.scrub.map(self.finish_at.map_or(t, |f| t.min(f)));
        let on_trace = |s: f64| g.x + g.w * (1.0 - (t_eff - s) / WINDOW).clamp(0.0, 1.0) as f32;
        let cancelled = self.mode == Mode::Cancelled;
        for (i, f) in words.iter().enumerate() {
            let (k, w) = (k[i], widths[i]);
            // Where it lands, and where its sound is on the trace now.
            let (tx0, tx1, ty, ta) = (rights[i] - w, rights[i], y0 + h / 2.0, h / 2.0 - 3.0);
            let (sx0, sx1) = (on_trace(f.sound.0), on_trace(f.sound.1));
            let (sy, sa) = (g.y + g.h / 2.0, g.h / 2.0 - 4.0);
            let mix = |a: f32, b: f32| a + (b - a) * k;
            let (x0, x1, cy, amp) = (mix(sx0, tx0), mix(sx1, tx1), mix(sy, ty), mix(sa, ta));
            // Words pushed off the left fade out.
            let alpha = ((x0 - g.x) / 24.0).clamp(0.0, 1.0);
            if alpha <= 0.0 || x1 - x0 < 1.0 {
                continue;
            }
            let n = ((x1 - x0) / 1.5).max(4.0) as usize;
            let pts: Vec<Point> = (0..n)
                .map(|j| {
                    let u = j as f32 / (n - 1) as f32;
                    let e = f.env[((u * (f.env.len() - 1) as f32).round() as usize).min(f.env.len() - 1)];
                    let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
                    let z = noise(j as i64 + f.id as i64 * 977);
                    Point::new(x0 + u * (x1 - x0), cy - sign * (0.08 + 0.92 * z * z) * e * amp)
                })
                .collect();
            let colour = if cancelled { hex(DIM) } else { hex(TRACE) };
            let line = Contour::polyline(&pts, false);
            p.stroke(&line, &StrokeStyle::solid(1.0, colour.fade((0.45 + 0.5 * k) * alpha)));
            // The word tick, bright for a moment after it lands.
            if k > 0.0 {
                let fresh = if *reduced { 0.0 } else { 1.0 - ((t - f.at - STRIP_FLY) / STRIP_TICK).clamp(0.0, 1.0) };
                let tick = if cancelled {
                    hex(DIM)
                } else {
                    let (a, b, k) = (hex(TRACE).fade(0.45), hex(VALUE), fresh as f32);
                    let m = |x: f32, y: f32| x + (y - x) * k;
                    Color::rgba(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b), m(a.a, b.a))
                };
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(tx0 - 2.0, y0 + 2.0, 1.0, h - 4.0), 0.0),
                    Paint::solid(tick.fade(k * alpha)),
                ));
            }
        }
    }

    /// The segment meters under the readouts.
    fn draw_meters(&self, p: &mut Painter) {
        let r = scaled(READ);
        let x0 = r.x + 15.0 * S;
        let w = r.w - 30.0 * S;
        let rows = [(108.0 * S, 14usize), (172.0 * S, 10), (234.0 * S, 10)];
        for (m, (y, n)) in rows.iter().enumerate() {
            let pitch = w / *n as f32;
            if m > 0 {
                p.shape(
                    ShapeInstance::rrect(
                        RRect::new(Rect::new(x0, *y - 0.5, w, 6.0 * S), 1.0),
                        Paint::NONE,
                    )
                    .with_stroke(Paint::solid(hex(OFF)), 1.0),
                );
            }
            for i in 0..*n {
                let lit = i < self.meters[m];
                if m > 0 && !lit {
                    continue;
                }
                p.shape(ShapeInstance::rrect(
                    RRect::new(
                        Rect::new(x0 + i as f32 * pitch, *y, pitch - 1.5, 5.0 * S),
                        0.5,
                    ),
                    Paint::solid(if lit { hex(VALUE) } else { hex(OFF) }),
                ));
            }
        }
    }

    fn draw_icons(&self, t: f64, p: &mut Painter) {
        let mic = |c: Point, scale: f32, colour: Color, p: &mut Painter| {
            let o = Outline::new(&icons::mic(), 0.05).transform(
                &Affine::translate(-12.0, -12.0)
                    .then(&Affine::scale(scale))
                    .then(&Affine::translate(c.x, c.y)),
            );
            if let Some(head) = o.contours.first() {
                p.fill([head], &StrokeStyle::solid(0.0, colour));
            }
            for k in o.contours.iter().skip(1) {
                p.stroke(k, &StrokeStyle::solid(1.4, colour));
            }
        };
        mic(Point::new(27.0 * S, 24.0 * S), 0.85, hex(TEXT), p);
        // The activity dots at the top right blink with the voice.
        let live = self.mode == Mode::Listening;
        for i in 0..6 {
            let (col, row) = (i % 3, i / 3);
            let c = Point::new((862.0 + col as f32 * 11.0) * S, (19.0 + row as f32 * 11.0) * S);
            let on = live
                && !reduced()
                && noise(((t - self.shown_at) * 6.0) as i64 * 7 + i as i64) > 0.4;
            let colour = if on || (i == 0 && self.mode != Mode::Cancelled) {
                hex(VALUE)
            } else {
                hex(DIM)
            };
            p.shape(ShapeInstance::circle(c, 2.6 * S, Paint::solid(colour)));
        }
        // The status mic in its dark disc.
        let c = Point::new(323.0 * S, STATUS_Y * S);
        p.shape(ShapeInstance::circle(c, 11.0 * S, Paint::solid(hex(0x0d3a26))));
        mic(c, 0.55, hex(VALUE), p);
    }

    fn draw_caret(&self, t: f64, p: &mut Painter) {
        let Some((c, since)) = self.caret else {
            return;
        };
        let idle = t - since - 0.4;
        let on = idle < 0.0 || reduced() || (idle * 1.1).fract() < 0.55;
        if on {
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(c.x, c.y, 3.0, LINE * 0.62), 0.0),
                Paint::solid(hex(TEXT)),
            ));
        }
        // MO-73: the caret stands on a foot that each onset splits.
        if let Some(onsets) = &self.joint {
            let gap = sounding::joint_gap(onsets, t, reduced());
            sounding::draw_foot(
                p,
                gap,
                c.x + 1.5,
                c.y + LINE * 0.62 + 1.0,
                12.0,
                2.0,
                hex(TEXT),
            );
        }
    }
}

impl Popup {
    fn scope_mode(&self) -> Mode {
        let status = self.status();
        if self.extra.final_at.is_some() {
            Mode::Done
        } else if self.ended() {
            if self.extra.finish_at.is_some() {
                Mode::Done
            } else {
                Mode::Cancelled
            }
        } else if status.loading {
            Mode::Loading
        } else if status.finishing || self.extra.finish_at.is_some() {
            Mode::Finishing
        } else {
            Mode::Listening
        }
    }

    /// The input level in dBFS from the newest levels, or `None` when quiet.
    fn scope_dbfs(&self) -> Option<f32> {
        // The levels are `tick_level`'s meter fraction to the power 1.5,
        // after the visual gain; undo both.
        let peak = self.levels.iter().rev().take(25).copied().fold(0.0f64, f64::max);
        (peak > 1e-3).then(|| {
            let f = peak.powf(2.0 / 3.0);
            (60.0 * f - 60.0 - 20.0 * (self.look.gain / 10.0).log10()) as f32
        })
    }

    pub(crate) fn sync_scope(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Scope {
            return;
        }
        let mode = self.scope_mode();
        let dbfs = self.scope_dbfs();
        let elapsed = (t - self.shown_at).max(1.0) / 60.0;
        let wpm = (self.word_count as f64 / elapsed).round() as u32;
        let s = &mut self.scope;
        if s.mode != Some(mode) {
            if s.mode.is_some() {
                s.mode_at = t;
            }
            s.mode = Some(mode);
        }
        let f = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if (f - s.prog.1).abs() > 1e-4 {
            let cur = s.prog.0 + (s.prog.1 - s.prog.0) * ease_out((t - s.prog.2) / 0.35);
            s.prog = (cur, f, t);
        }
        // Readouts change at most twice a second, and freeze at the stop.
        if mode == Mode::Listening && t - s.read_at >= 0.5 {
            s.dbfs = dbfs;
            s.wpm = wpm;
            s.read_at = t;
        }
        if self.word_count != s.words {
            s.words = self.word_count;
            s.word_at = t;
        }
        if self.look.on(SENTENCE_TRACE) {
            self.sync_sentence(t);
        }
    }

    /// MO-63: freezes each new word's stretch of trace once its sound is
    /// all in, and lets go of the words that went.
    fn sync_sentence(&mut self, t: f64) {
        let words = self.model.words();
        let from = words.len().saturating_sub(STRIP_WORDS);
        let mut kept = Vec::with_capacity(words.len() - from);
        for w in &words[from..] {
            if let Some(f) = self.scope.sentence.iter().find(|f| f.id == w.id) {
                kept.push(f.clone());
                continue;
            }
            let Some(h) = self.heard.word(w.id) else {
                continue;
            };
            let (a, b) = h.sound;
            if b > self.newest_at || b <= a {
                continue;
            }
            kept.push(Frozen {
                id: w.id,
                sound: h.sound,
                at: t,
                env: envelope(&self.wave, self.newest_at, a, b, STRIP_POINTS).into(),
            });
        }
        self.scope.sentence = kept;
    }

    pub(crate) fn arm_scope_tick(&mut self, _cx: &mut Cx<Msg>) {}

    /// m18: whether the idle motion holds still: reduced motion, or the
    /// popup is closing.
    fn scope_idle_still(&self) -> bool {
        reduced() || self.look.on("reduced-motion") || self.closing
    }

    /// m18: the graticule glow and the trigger marker, each with its idle
    /// loop while the popup is open, at rest under reduced motion. Offset
    /// by `o`, the card margin.
    fn scope_idle(&self, o: f32) -> [Element<Msg>; 2] {
        let still = self.scope_idle_still();
        let size = Self::scope_surface(&self.look);
        let lines = grid_lines(self.look.on(SENTENCE_TRACE));
        let glow = at(
            Element::canvas("scope-glow", 0.0, move |_, b, p| {
                // The lines again in the trace's colour, faded by the node's opacity.
                for r in &lines {
                    let (x, y) = (b.x + r.x + o, b.y + r.y + o);
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(x, y, r.w, r.h), 0.0),
                        Paint::solid(hex(TRACE)),
                    ));
                }
            }),
            0.0,
            0.0,
        )
        .size(size.w, size.h)
        .opacity(GLOW_ALPHA);
        // On the graticule's left edge at the centre line: the trigger level.
        let g = graticule();
        let (tw, th) = (6.0, 9.0);
        let marker = at(
            Element::canvas("scope-trigger", 0.0, move |_, b, p| {
                let tri = Contour::polyline(
                    &[Point::new(b.x, b.y), Point::new(b.x + tw, b.y + th / 2.0), Point::new(b.x, b.y + th)],
                    true,
                );
                p.fill([&tri], &StrokeStyle::solid(0.0, hex(VALUE)));
            }),
            o + g.x + 1.0,
            o + g.y + g.h / 2.0 - th / 2.0 + 0.5,
        )
        .size(tw, th);
        if still {
            return [glow, marker];
        }
        let breathe = Loop::pulse(GLOW_SWING, GLOW_PERIOD).fps(IDLE_FPS).settle(Spring::SNAP);
        let blink = Loop {
            amp: -(1.0 - TRIGGER_DIM) as f64,
            ..Loop::gate(0.5, 1.0, 1.0).fps(IDLE_FPS).settle(Spring::SNAP)
        };
        [glow.ambient(LoopProp::Opacity, breathe), marker.ambient(LoopProp::Opacity, blink)]
    }

    pub(crate) fn scope_surface(_look: &Look) -> Size {
        Size::new((W + 2.0 * MARGIN).ceil(), (H + 2.0 * MARGIN).ceil())
    }

    pub(crate) fn scope_view(&self) -> Element<Msg> {
        let size = Self::scope_surface(&self.look);
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        root.child(self.scope_panel(size, 0.0, 0.0))
    }

    fn scope_panel(&self, size: Size, x: f32, y: f32) -> Element<Msg> {
        let l = &self.look;
        let s = &self.scope;
        let mode = s.mode.unwrap_or(Mode::Loading);
        let f = fonts(&l.fonts);
        let o = MARGIN;
        let style = l.text_style();
        let all = self.stable_lines(&style, WRAP);
        let lines = &all[all.len().saturating_sub(MAX_LINES as usize)..];

        let caret = (mode == Mode::Listening).then(|| {
            let (row, w) = lines
                .last()
                .map_or((0usize, 0.0), |(_, w)| (lines.len() - 1, *w));
            (
                Point::new(TEXT_X + w + 8.0, TEXT_TOP + row as f32 * LINE + LINE * 0.18),
                s.word_at,
            )
        });
        let level_segs = s
            .dbfs
            .map_or(0, |d| (((d + 60.0) / 60.0).clamp(0.0, 1.0) * 14.0).round() as usize);
        let secs = (self.extra.now - self.shown_at).max(0.0);
        let scene = Scene {
            mode,
            mode_at: s.mode_at,
            shown_at: self.shown_at,
            levels: self.wave.clone(),
            newest_at: self.newest_at,
            scrub: self.scrub(),
            finish_at: self.extra.finish_at,
            final_at: self.extra.final_at,
            prog: (mode == Mode::Finishing && self.files.fraction.is_some()).then_some(s.prog),
            meters: [
                level_segs,
                ((secs / 6.0).min(10.0)) as usize,
                ((s.wpm as f32 / 20.0).min(10.0)) as usize,
            ],
            caret: caret.map(|(c, at)| (Point::new(c.x + o, c.y + o), at)),
            joint: self.joint_onsets(),
            sentence: l
                .on(SENTENCE_TRACE)
                .then(|| (Arc::from(s.sentence.as_slice()), reduced() || l.on("reduced-motion"))),
            room: tick_level(10f64.powf(ROOM_TONE_DBFS / 20.0), l.gain) as f32,
            still: self.scope_idle_still(),
        };
        let until = scene.until(self.extra.now);
        let canvas = at(
            Element::canvas("scope", until, move |t, r, p| scene.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h);

        let mono = |px: f32| TextStyle::new(f.mono, px, px * 1.3).tracking(0.6);
        let bold = |px: f32| TextStyle::new(f.bold, px, px * 1.3).tracking(0.6);
        let label = |key: String, text: String, style: TextStyle, colour: Color, px: f32, py: f32| {
            at(
                Element::label(key, text).text_style(style).color(colour),
                o + px * S,
                o + py * S,
            )
        };
        let mut panel = at(Element::layer("scope"), x, y)
            .size(size.w, size.h)
            .presence(Transition::fade(), Transition::fade().scale(0.98))
            .child(canvas)
            .children(self.scope_idle(o))
            .child(label("title".into(), "VOICE DICTATION".into(), bold(14.0 * S * 1.15), hex(TEXT), 61.0, 15.0))
            .child(
                label(
                    format!("mode-{}", mode.name()),
                    mode.name().into(),
                    mono(11.0 * S * 1.2),
                    hex(LABEL),
                    845.0 - self.measure.layout(mode.name(), &mono(11.0 * S * 1.2), None).size.w / S,
                    17.0,
                )
                .presence(Transition::fade(), Transition::fade()),
            )
            .child(label("live".into(), "LIVE TRANSCRIPT".into(), bold(13.0 * S * 1.2), hex(0xc6f3dc), 28.0, 309.0))
            .child(label("en".into(), "EN".into(), bold(12.0 * S * 1.2), hex(0xc6f3dc), 868.0, 309.0));
        // The dBFS scale.
        for (i, mark) in ["0 dBFS", "-12", "-24", "-36", "-48", "-60"].iter().enumerate() {
            panel = panel.child(label(
                format!("scale{i}"),
                (*mark).into(),
                mono(12.0 * S * 1.2),
                hex(0xb9e9cf),
                23.0,
                68.0 + i as f32 * 34.6,
            ));
        }
        // The readouts: name, value and unit, with the digits rolling.
        let roll = Spring::critical_to_98(0.25);
        let name = |key: &'static str, text: &'static str, py: f32| {
            label(key.into(), text.into(), mono(10.5 * S * 1.25), hex(LABEL), 760.0, py)
        };
        let value = |key: &'static str, text: String, py: f32| {
            at(
                digit_roll(key, &text, mono(17.0 * S * 1.25), hex(VALUE), roll),
                o + 760.0 * S,
                o + py * S,
            )
        };
        let dbfs = s.dbfs.map_or("--.- dBFS".to_string(), |d| format!("{d:.1} dBFS"));
        panel = panel
            .child(name("n-level", "INPUT LEVEL", 66.0))
            .child(value("v-level", dbfs, 81.0))
            .child(name("n-time", "ELAPSED", 131.0))
            .child(value("v-time", format!("{} s", secs as u32), 146.0))
            .child(name("n-wpm", "WORDS / MIN", 193.0))
            .child(value("v-wpm", format!("{} wpm", s.wpm), 208.0))
            .child(name("n-words", "WORDS", 253.0))
            .child(value("v-words", format!("{}", self.word_count), 268.0));

        // The transcript lines, one live text each so a line keeps its place.
        let colour = if mode == Mode::Cancelled { hex(DIM) } else { hex(TEXT) };
        let rw = self.rewinding();
        if l.on(wild_space::KINETIC_LATTICE) {
            // The kinetic lattice under each line, the times on the newest.
            for (i, (words, _)) in lines.iter().enumerate() {
                let newest = i + 1 == lines.len();
                let look = vo::scene::lattice::LatticeLook {
                    bow: (1.5, 8.0),
                    bow_per_sec: 22.0,
                    drop: 4.0,
                    stem: 6.0,
                    color: hex(TRACE).fade(0.5),
                    accent: hex(VALUE),
                    label: newest.then(|| mono(LATTICE_LABEL_PX)),
                    label_color: hex(LABEL).fade(0.8),
                    ..Default::default()
                };
                panel = panel.child(lattice_rewind(at(
                    self.lattice_line(
                        format!("lattice-{}", words[0].id),
                        words,
                        &style,
                        Size::new(WRAP + 40.0, LINE + 14.0),
                        &look,
                        // LT-04: the live strand ends at the caret's foot, which
                        // the scene draws a margin further in than `caret`.
                        caret.filter(|_| newest).map(|(c, _)| c.x - TEXT_X + MARGIN + 1.5),
                        // LT-09: only older lines drift, inside their own line.
                        (!newest).then_some(LINE),
                    ),
                    o + TEXT_X,
                    o + TEXT_TOP + i as f32 * LINE,
                ), rw, reduced() || l.on("reduced-motion")));
            }
        }
        for (i, (words, _)) in lines.iter().enumerate() {
            panel = panel.child(
                at(
                    Element::live_text(format!("line-{}", words[0].id), words)
                        .text_style(style)
                        .color(colour)
                        .motion(Prop::Opacity, Spring::critical_to_98(0.08))
                        .word_enter(Transition::fade()),
                    o + TEXT_X,
                    o + TEXT_TOP + i as f32 * LINE,
                )
                .motion(Prop::Position, Spring::critical_to_98(0.12))
                .rewind_hide(rw)
                .presence(Transition::fade(), Transition::fade()),
            );
        }
        // MO-50: a cancel takes the words back into the caret.
        if rw {
            let at_line: Vec<(&[Word], Point)> = lines
                .iter()
                .enumerate()
                .map(|(i, (w, _))| (w.as_slice(), Point::new(o + TEXT_X, o + TEXT_TOP + i as f32 * LINE)))
                .collect();
            let ink = endings::Ink {
                text: colour,
                mark: hex(TRACE),
                faint: hex(DIM),
            };
            panel = panel.children(self.rewind_mark("rewind", self.lay_lines(&at_line, style), ink, size));
        }

        // MO-71 receipts and MO-73 joint ticks under the newest line, and
        // the MO-72 pitch score in the margin beside it.
        if let Some((words, _)) = lines.last() {
            let (x, y) = (o + TEXT_X, o + TEXT_TOP + (lines.len() - 1) as f32 * LINE);
            // In the panel's right margin, past where lines wrap.
            let sx = o + (INNER.0 + INNER.2) * S - 30.0;
            let (ink, score) = (hex(VALUE), Rect::new(sx, y + LINE * 0.1, 24.0, LINE * 0.6));
            panel = panel
                .children(self.receipts_element("scope", words, &style, x, y, WRAP, ink))
                .children(self.joint_ticks_element("scope", words, &style, x, y, WRAP, ink, false))
                .children(self.score_element(score, hex(TRACE)));
        }

        // The status row.
        let status = self.status();
        let text = match mode {
            Mode::Done => {
                let app = self
                    .files
                    .live
                    .as_ref()
                    .and_then(|d| d.target.as_ref())
                    .map(|t| t.app.clone())
                    .filter(|a| !a.is_empty());
                {
                    let word = self.delivery_word().to_uppercase();
                    app.map_or(word.clone(), |a| format!("{word}  ·  into {a}"))
                }
            }
            Mode::Cancelled => "CANCELLED".to_string(),
            _ => format!("{}{}", status.state, status.detail),
        };
        panel.child(
            label(
                format!("status-{}", status.state),
                text,
                mono(13.0 * S * 1.2),
                hex(0xc6f3dc),
                343.0,
                STATUS_Y - 9.0,
            )
            .presence(Transition::fade(), Transition::fade()),
        )
    }
}

#[cfg(test)]
mod sentence_tests {
    use super::*;

    #[test]
    fn landed_words_pack_from_the_right_without_overlap() {
        let widths = [20.0, 8.0, 30.0];
        let rights = slot_rights(&widths, &[1.0, 1.0, 1.0], 500.0);
        assert_eq!(rights[2], 500.0);
        for i in 0..2 {
            let gap = (rights[i + 1] - widths[i + 1]) - rights[i];
            assert!((gap - STRIP_GAP).abs() < 1e-4, "gap {gap}");
        }
    }

    #[test]
    fn older_words_make_room_only_as_the_new_one_lands() {
        let widths = [20.0, 8.0, 30.0];
        let before = slot_rights(&widths[..2], &[1.0, 1.0], 500.0);
        let starting = slot_rights(&widths, &[1.0, 1.0, 0.0], 500.0);
        assert_eq!(&starting[..2], &before[..]);
        let half = slot_rights(&widths, &[1.0, 1.0, 0.5], 500.0);
        assert!((before[1] - half[1] - 0.5 * (30.0 + STRIP_GAP)).abs() < 1e-4);
    }

    #[test]
    fn a_frozen_envelope_follows_the_levels_it_came_from() {
        // One second of levels at FRAME_HZ: quiet, then loud.
        let n = FRAME_HZ as usize;
        let levels: Vec<f32> = (0..n).map(|i| if i < n / 2 { 0.0 } else { FULL }).collect();
        let env = envelope(&levels, 1.0, 0.1, 0.9, 9);
        assert_eq!(env.len(), 9);
        assert_eq!(env[0], 0.0);
        assert!((env[8] - 1.0).abs() < 1e-6);
        // Sound from before the levels start reads as silence.
        assert!(envelope(&levels, 1.0, -5.0, -4.0, 4).iter().all(|&e| e == 0.0));
        assert!(strip_width((0.0, 0.01)) >= STRIP_MIN_W);
    }

    #[test]
    fn the_live_floor_follows_the_room_and_stays_within_a_pixel() {
        let room = 0.125;
        assert_eq!(live_floor(0.0, room), 0.0, "true silence draws flat");
        assert_eq!(live_floor(room, room), FLOOR_PX);
        assert_eq!(live_floor(0.9, room), FLOOR_PX, "never more than FLOOR_PX");
        let quiet = live_floor(room / 4.0, room);
        assert!(quiet > 0.0 && quiet < FLOOR_PX);
        assert_eq!(live_floor(room, 0.0), 0.0, "off");
    }

    #[test]
    fn the_grid_glow_lines_stay_in_the_graticule_and_above_the_strip() {
        let g = graticule();
        let foot = g.y + g.h - STRIP_H * S;
        for strip in [false, true] {
            let lines = grid_lines(strip);
            assert!(!lines.is_empty());
            for r in lines {
                assert!(r.x >= g.x - 0.01 && r.x + r.w <= g.x + g.w + 1.01, "{r:?}");
                assert!(r.y >= g.y - 0.01 && r.y + r.h <= g.y + g.h + 1.01, "{r:?}");
                if strip {
                    assert!(r.y + r.h <= foot + 1.01, "{r:?} reaches into the strip");
                }
            }
        }
    }

    #[test]
    fn the_idle_motion_stays_out_of_the_transcript() {
        // Everything m18 moves is in the graticule, well above the first text line.
        let g = graticule();
        assert!(g.y + g.h < TEXT_TOP);
    }
}
