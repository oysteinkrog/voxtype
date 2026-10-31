//! The `phosphor` look of the vo popup: a green CRT terminal. A rounded,
//! slightly bowed screen with scanlines holds a header with the microphone,
//! "VOICE DICTATION", an elapsed clock and a REC light; a meter of stacked
//! phosphor blocks; the transcript in a monospaced face with a block caret;
//! and an italic status line.
//!
//! New words are written by a bright 3 px beam head that sweeps each word in
//! 80 ms, leaving the glyphs lit behind it (MO-61). The caret glides to the
//! end of each new word, stays solid while words come and blinks once they
//! stop. A revision flashes only the changed letters in reverse video, and
//! the beam rewrites just those letters. The meter's columns
//! light whole blocks, the newest level in the middle and older ones running
//! out to both sides; once the recording stops they sink to a row of dots
//! and an ASCII progress bar counts up the final pass.
//!
//! The redesign's other three effects play in the same material (d6x).
//! With `stroke-write` the pen writes each new word in green wet ink with a
//! pale nib light and rewrites a revised word's changed letters, so the
//! beam stays off; without the pen the beam writes as above.
//! With `confidence-ink` a tentative word sits at 75 % ink over a row of
//! green phosphor dots with a gap, which fills and fades once the word
//! sets. With `broken-frame` a failure snaps the screen's green border open
//! on its top edge, the ends lit pale, with the failed stage beside the
//! break. Under reduced motion there is no beam and no pen, the seam stays
//! and the broken frame is a still mark.
//!
//! While the popup is open and listening it keeps a little idle life (5ti,
//! RD-02): a 2 px scanline a little brighter than the screen rolls down it
//! once every 5 s, the block caret blinks 530 ms on and 530 ms off, and a
//! faint static flickers on the screen's background 12 times a second.
//! Neither the scanline nor the static enters the newest line's box. All
//! stop once the popup closes, and under reduced motion the caret stays
//! solid and nothing rolls or flickers.
//!
//! Geometry is laid out in the pixels of the concept image and scaled so the
//! screen is as wide as the popup.

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Outline, StrokeStyle};

use super::glyph_paths::{BrokenPen, PathsText};
use vo::scene::roll::digit_roll;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Width of the screen in the concept, in concept pixels.
const CONCEPT_W: f32 = 728.0;
/// Room round the screen for its glow, in logical pixels.
const MARGIN: f32 = 16.0;

// Layout down the screen, in concept pixels from its top edge.
const PAD_X: f32 = 37.0;
const HEADER_Y: f32 = 49.0;
const RULE_Y: f32 = 74.0;
const METER_BASE: f32 = 146.0;
const TEXT_TOP: f32 = 168.0;
const LINE: f32 = 49.0;
const TEXT_PX: f32 = 34.0;
const RULE2_GAP: f32 = 30.0;
const STATUS_GAP: f32 = 38.0;
const BOTTOM_PAD: f32 = 42.0;
const RADIUS: f32 = 22.0;
const LABEL_PX: f32 = 18.5;
const STATUS_PX: f32 = 19.0;

/// Columns in the block meter, and blocks in its tallest column.
const COLUMNS: usize = 49;
const BLOCKS: usize = 11;
/// Block height and the gap under it, in concept pixels.
const BLOCK_H: f32 = 4.2;
const BLOCK_GAP: f32 = 1.9;
/// Seconds of level history from the middle of the meter to each end.
const HISTORY_SECS: f32 = 2.2;
/// Seconds for the meter to sink to dots once the recording stops.
const SINK_SECS: f64 = 0.4;

/// The caret: solid this long after the last new word, then blinking
/// 530 ms on and 530 ms off (5ti).
const CARET_HOLD: f64 = 0.4;
const CARET_BLINK: f64 = 0.53;
const CARET_GLIDE: f64 = 0.12;
/// 5ti: the idle motion's pace (RD-02's slow-loop cap); the scanline's
/// period, pass, first pass after the popup opens, height and alpha; the
/// static's rate, specks and alpha (in linear light, like every blend here:
/// a few percent of green lights the dark screen far more than it says).
const IDLE_FPS: f64 = 15.0;
const ROLL_EVERY: f64 = 5.0;
const ROLL_PASS: f64 = 1.6;
const ROLL_FIRST: f64 = 2.0;
const ROLL_H: f32 = 2.0;
const ROLL_ALPHA: f32 = 0.06;
const NOISE_FPS: f64 = 12.0;
const NOISE_SPECKS: u32 = 40;
const NOISE_ALPHA: f32 = 0.02;
/// Seconds a revised word stays in reverse video.
const FLASH_SECS: f64 = 0.35;
/// The beam: seconds to write a whole word, the head's width in logical
/// pixels and how long its afterglow trails.
const BEAM_SECS: f64 = 0.08;
const BEAM_W: f32 = 3.0;
const BEAM_TAIL: f64 = 0.05;

const GREEN: u32 = 0x42f9a8;
const HOT: u32 = 0xa6ffdc;
const MID: u32 = 0x22c67a;
const DIM: u32 = 0x0f6e43;
const SCREEN: u32 = 0x00170d;
const DEEP: u32 = 0x000c07;

fn hex(rgb: u32, a: f32) -> Color {
    Color::hex(rgb, a)
}

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

/// An element placed at `(x, y)` in its parent, out of the flow.
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

/// Fonts for the transcript, the captions and the status line.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    caption: u32,
    status: u32,
}

/// JetBrains Mono, the face closest to the concept's, or the look's own
/// fixed font when it is not installed. Added to the look's registry once
/// (`FontRegistry::system`), so every registry, such as a test's, has its
/// own ids.
fn fonts(look: &Look) -> Fonts {
    let add = |family: &str| look.fonts.system(family).unwrap_or(MONO);
    Fonts {
        text: add("JetBrains Mono:medium"),
        caption: add("JetBrains Mono:medium"),
        status: add("JetBrains Mono:medium italic"),
    }
}

/// Sizes in logical pixels, from the concept's.
#[derive(Clone, Copy)]
struct Geo {
    k: f32,
    w: f32,
}

impl Geo {
    fn new(look: &Look) -> Geo {
        Geo {
            k: look.width / CONCEPT_W,
            w: look.width,
        }
    }

    fn screen_h(&self, lines: u32) -> f32 {
        (TEXT_TOP + lines as f32 * LINE + RULE2_GAP + STATUS_GAP + BOTTOM_PAD) * self.k
    }

    fn surface(&self) -> Size {
        let h = self.screen_h(swiss::MAX_LINES as u32) + 2.0 * MARGIN;
        Size::new((self.w + 2.0 * MARGIN).ceil(), h.ceil())
    }
}

/// State only the phosphor look keeps.
#[derive(Default)]
pub struct State {
    /// When the dictation ended without a final text.
    ended_at: Option<f64>,
    /// The caret's last target, the one before and when it moved.
    caret: Option<(Point, Point, f64)>,
    /// The words on screen last time, by id, to spot a revised one.
    seen: Vec<(u32, String)>,
    /// The latest revised word and when it changed.
    flash: Option<(u32, f64)>,
}

/// The status line's icon.
#[derive(Clone, Copy, PartialEq)]
enum Icon {
    /// Five bars that follow the level.
    Bars,
    /// A slash that turns like a terminal spinner.
    Spinner,
    /// Nothing: a check or a cross is drawn as its own element.
    None,
}

/// What the screen canvas draws, fixed for one view.
struct Screen {
    k: f32,
    w: f32,
    shown_at: f64,
    /// When the meter starts to sink.
    sink_at: Option<f64>,
    loading: bool,
    wave: Arc<[f32]>,
    newest_at: f64,
    rate: f32,
    /// The meter's clock during a rewind cancel (MO-50).
    scrub: endings::Scrub,
    /// Where the caret goes, where it came from and when it left, or `None`
    /// when no caret shows.
    caret: Option<(Point, Point, f64)>,
    /// The revised word's box and when it changed.
    flash: Option<(Rect, f64)>,
    /// The REC light: on while recording.
    rec: bool,
    /// No light at all: a check or a cross stands in its place.
    no_light: bool,
    /// The status icon's bars follow the level while listening.
    level: f32,
    /// What the status icon shows.
    icon: Icon,
    status_y: f32,
    rule2_y: f32,
    /// 5ti: the idle scanline and static run (open, listening, not
    /// reduced), and the newest line's box (top and bottom), which both
    /// keep out of.
    idle: bool,
    newest: (f32, f32),
    /// The screen's height, and reduced motion (the caret stays solid).
    h: f32,
    still: bool,
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// 5ti: the rolling scanline's top as a share of the screen's height `s`
/// seconds after the popup opened, or `None` between passes.
fn roll_at(s: f64) -> Option<f32> {
    let u = (s - ROLL_FIRST).rem_euclid(ROLL_EVERY) / ROLL_PASS;
    (s >= ROLL_FIRST && u < 1.0).then_some(u as f32)
}

/// 5ti: the block caret shows `idle` seconds after it settled: on for the
/// first `CARET_BLINK` of each pair, off for the second.
fn caret_on(idle: f64) -> bool {
    idle < 0.0 || (idle / CARET_BLINK).floor() as i64 % 2 == 0
}

impl Screen {
    /// 5ti: the static and the rolling scanline, under everything else and
    /// clear of the newest line's box.
    fn draw_idle(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let (top, bottom) = self.newest;
        let clear = |y0: f32, y1: f32| y1 < top || y0 > bottom;
        let (x0, x1) = (PAD_X * k * 0.5, self.w - PAD_X * k * 0.5);
        let (y_min, y_max) = (8.0 * k, self.h - 8.0 * k);
        let n = ((t - self.shown_at) * NOISE_FPS).floor() as i64 as u32;
        let r = 0.9 * k;
        for i in 0..NOISE_SPECKS {
            let seed = n.wrapping_mul(7919) ^ i.wrapping_mul(0x2545_F491);
            let x = x0 + noise(seed) * (x1 - x0);
            let y = y_min + noise(seed.wrapping_mul(0x6C8E_9CF5) ^ 0x1234_5679) * (y_max - y_min);
            if clear(y - r, y + r) {
                p.shape(ShapeInstance::circle(
                    Point::new(x, y),
                    r,
                    Paint::solid(hex(GREEN, NOISE_ALPHA)),
                ));
            }
        }
        let s = (t * IDLE_FPS).floor() / IDLE_FPS - self.shown_at;
        if let Some(u) = roll_at(s) {
            let y = y_min + u * (y_max - y_min - ROLL_H * k);
            if clear(y, y + ROLL_H * k) {
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x0, y, x1 - x0, ROLL_H * k), 0.0),
                    Paint::solid(hex(GREEN, ROLL_ALPHA)),
                ));
            }
        }
    }

    /// The level shown at `age` seconds before now, 0 to 1.
    fn level_at(&self, t: f64, age: f32) -> f32 {
        let t = self.scrub.map(t);
        let n = self.wave.len();
        if n == 0 {
            return 0.0;
        }
        let i = (n - 1) as f32 - ((self.newest_at - t) as f32 + age) * self.rate;
        if i < 0.0 {
            return 0.0;
        }
        let i = i.min((n - 1) as f32);
        let (a, b) = (i.floor() as usize, i.ceil() as usize);
        let f = i - a as f32;
        self.wave[a] * (1.0 - f) + self.wave[b] * f
    }

    /// Whole blocks lit in meter column `i` at `t`.
    fn blocks(&self, t: f64, i: usize) -> usize {
        let mid = (COLUMNS - 1) as f32 / 2.0;
        let d = (i as f32 - mid).abs() / mid;
        let seed = ((i as f32 * 12.9898).sin() * 43758.547).fract().abs();
        let weight = 0.45 + 0.55 * seed;
        let mut v = self.level_at(t, d * HISTORY_SECS) * weight * (1.0 - 0.9 * d.powf(0.8));
        if let Some(s) = self.sink_at {
            v *= 1.0 - ease_out((t - s) / SINK_SECS);
        }
        (v * 1.7 * BLOCKS as f32).floor().clamp(0.0, BLOCKS as f32) as usize
    }

    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let x0 = PAD_X * k;
        let x1 = self.w - PAD_X * k;
        let solid = |c: u32, a: f32| Paint::solid(hex(c, a));
        let rect = |x: f32, y: f32, w: f32, h: f32, paint: Paint| {
            ShapeInstance::rrect(RRect::new(Rect::new(x, y, w, h), 0.6 * k), paint)
        };

        if self.idle {
            self.draw_idle(t, p);
        }

        // The rules under the header and over the status line.
        for y in [RULE_Y * k, self.rule2_y] {
            p.shape(ShapeInstance::line(
                Point::new(x0, y),
                Point::new(x1, y),
                1.6 * k,
                solid(MID, 0.85),
            ));
        }

        // The block meter.
        let pitch = (x1 - x0) / COLUMNS as f32;
        let bw = pitch * 0.62;
        let base = METER_BASE * k;
        for i in 0..COLUMNS {
            let x = x0 + i as f32 * pitch + (pitch - bw) / 2.0;
            let lit = if self.loading { 0 } else { self.blocks(t, i) };
            if lit == 0 {
                let dot = 3.4 * k;
                p.shape(rect(
                    x + (bw - dot) / 2.0,
                    base - dot,
                    dot,
                    dot,
                    solid(GREEN, 0.75),
                ));
                continue;
            }
            for b in 0..lit {
                let y = base - (b as f32 + 1.0) * (BLOCK_H + BLOCK_GAP) * k + BLOCK_GAP * k;
                let top = b + 1 == lit;
                let a = if top { 1.0 } else { 0.82 };
                p.shape(rect(
                    x,
                    y,
                    bw,
                    BLOCK_H * k,
                    solid(if top { HOT } else { GREEN }, a),
                ));
            }
        }
        if self.loading {
            // A bright block runs to and fro over the dots.
            let phase = ((t - self.shown_at) * 0.9).rem_euclid(2.0);
            let f = if phase < 1.0 { phase } else { 2.0 - phase } as f32;
            let at = f * (COLUMNS - 1) as f32;
            for i in 0..COLUMNS {
                let near = (1.0 - (i as f32 - at).abs() / 4.0).max(0.0);
                if near <= 0.0 {
                    continue;
                }
                let x = x0 + i as f32 * pitch + (pitch - bw) / 2.0;
                let n = (near * 3.0).ceil() as usize;
                for b in 0..n {
                    let y = base - (b as f32 + 1.0) * (BLOCK_H + BLOCK_GAP) * k + BLOCK_GAP * k;
                    p.shape(rect(x, y, bw, BLOCK_H * k, solid(GREEN, near)));
                }
            }
        }

        // The REC light breathes once a second.
        let rec_c = Point::new(self.w - PAD_X * k - 47.0 * k, HEADER_Y * k + 1.0 * k);
        if self.no_light {
        } else if self.rec {
            let breath = 0.75 + 0.25 * (((t - self.shown_at) * std::f64::consts::TAU).cos() as f32);
            p.shape(ShapeInstance::circle(rec_c, 6.5 * k, solid(GREEN, breath)));
        } else {
            p.shape(
                ShapeInstance::circle(rec_c, 5.8 * k, Paint::NONE)
                    .with_stroke(solid(GREEN, 0.9), 1.6 * k),
            );
        }

        // The revised word in reverse video, fading back.
        if let Some((r, at)) = self.flash {
            let f = 1.0 - ease_out((t - at) / FLASH_SECS);
            if f > 0.0 {
                p.shape(rect(
                    r.x - 2.0 * k,
                    r.y,
                    r.w + 4.0 * k,
                    r.h,
                    solid(GREEN, 0.55 * f),
                ));
            }
        }

        // The block caret: glides to the new end, solid while words come,
        // then blinks.
        if let Some((to, from, moved)) = self.caret {
            let g = ease_out((t - moved) / CARET_GLIDE);
            let pos = Point::new(from.x + (to.x - from.x) * g, from.y + (to.y - from.y) * g);
            // Solid under reduced motion; the blink steps with the idle pace.
            let idle = (t * IDLE_FPS).floor() / IDLE_FPS - moved - CARET_HOLD;
            if self.still || caret_on(idle) {
                p.shape(rect(pos.x, pos.y, 15.0 * k, 37.0 * k, solid(GREEN, 1.0)));
            }
        }

        let cy = self.status_y;
        if self.icon == Icon::Spinner {
            // | / - \ at 8 steps a second.
            let step = ((t - self.shown_at) * 8.0).floor() as i64 % 4;
            let a = step as f32 * std::f32::consts::FRAC_PI_4;
            let (c, r) = (Point::new(x0 + 14.0 * k, cy), 11.0 * k);
            let d = Point::new(r * a.sin(), -r * a.cos());
            p.shape(ShapeInstance::line(
                Point::new(c.x - d.x, c.y - d.y),
                Point::new(c.x + d.x, c.y + d.y),
                3.0 * k,
                solid(GREEN, 1.0),
            ));
        }
        if self.icon != Icon::Bars {
            return;
        }
        // The status icon: five bars that follow the level while listening.
        let heights = [0.45, 0.75, 1.0, 0.75, 0.45];
        for (i, h) in heights.iter().enumerate() {
            let jig = ((t * 7.0 + i as f64 * 1.7).sin() as f32 * 0.5 + 0.5) * self.level;
            let hh = (h * (0.55 + 0.45 * jig.min(1.0)) * 26.0 * k).max(4.0 * k);
            let x = x0 + i as f32 * 7.0 * k;
            p.shape(ShapeInstance::capsule(
                Rect::new(x, cy - hh / 2.0, 3.2 * k, hh),
                solid(GREEN, 1.0),
            ));
        }
    }
}

/// One sweep of the beam: the line box it writes on, from and to which x,
/// when it starts and how long it takes. In the transcript's coordinates.
#[derive(Clone, Copy, Debug)]
struct Sweep {
    line: Rect,
    x0: f32,
    x1: f32,
    at: f64,
    secs: f64,
}

impl Sweep {
    /// Where the beam head is at `t`, or `None` before it starts and after
    /// it has written the last letter.
    fn head(&self, t: f64) -> Option<f32> {
        let s = t - self.at;
        if !(0.0..self.secs).contains(&s) {
            return None;
        }
        Some(self.x0 + (self.x1 - self.x0) * (s / self.secs) as f32)
    }
}

/// The beam that writes new words and rewrites changed letters (MO-61):
/// the letters it has not reached yet are covered with the screen's own
/// colour, and a bright head with a short afterglow sweeps over them.
#[derive(Clone)]
struct Beam {
    sweeps: Vec<Sweep>,
    k: f32,
    /// The transcript's top in the screen, and the screen's height, for the
    /// screen's gradient behind the cover.
    top: f32,
    screen_h: f32,
}

impl Beam {
    fn until(&self) -> f64 {
        self.sweeps
            .iter()
            .map(|s| s.at + s.secs + BEAM_TAIL)
            .fold(0.0, f64::max)
    }

    /// The screen's colour at transcript height `y`.
    fn screen_at(&self, y: f32) -> Color {
        let f = ((self.top + y) / self.screen_h).clamp(0.0, 1.0);
        let (a, b) = (hex(SCREEN, 1.0), hex(DEEP, 1.0));
        Color::rgba(
            a.r + (b.r - a.r) * f,
            a.g + (b.g - a.g) * f,
            a.b + (b.b - a.b) * f,
            1.0,
        )
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let pad = 6.0 * k;
        for s in &self.sweeps {
            let y = r.y + s.line.y;
            let h = s.line.h;
            let cover = |from: f32, p: &mut Painter| {
                let x = r.x + from;
                let w = s.x1 + pad - from;
                if w > 0.0 {
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(x, y, w, h), 0.0),
                        Paint::linear(
                            Point::new(0.0, y),
                            self.screen_at(s.line.y),
                            Point::new(0.0, y + h),
                            self.screen_at(s.line.y + h),
                        ),
                    ));
                }
            };
            let since = t - s.at;
            if since < 0.0 {
                // Its turn has not come: the letters wait unwritten.
                cover(s.x0, p);
                continue;
            }
            let (head, glow) = match s.head(t) {
                Some(x) => (x, 1.0),
                None => {
                    // The afterglow at the end of the word fades.
                    let g = 1.0 - ((since - s.secs) / BEAM_TAIL) as f32;
                    if g <= 0.0 {
                        continue;
                    }
                    (s.x1, g)
                }
            };
            if glow >= 1.0 {
                cover(head, p);
            }
            // The afterglow: the just-written letters still hot behind the
            // head.
            let tail = (s.x1 - s.x0).min(24.0 * k).max(BEAM_W * k);
            p.shape(ShapeInstance::rrect(
                RRect::new(
                    Rect::new(r.x + head - tail, y + 4.0 * k, tail, h - 6.0 * k),
                    0.0,
                ),
                Paint::linear(
                    Point::new(r.x + head - tail, 0.0),
                    hex(HOT, 0.0),
                    Point::new(r.x + head, 0.0),
                    hex(HOT, 0.28 * glow),
                ),
            ));
            let bar = Rect::new(
                r.x + head - BEAM_W * k / 2.0,
                y + 3.0 * k,
                BEAM_W * k,
                h - 4.0 * k,
            );
            let bar = RRect::new(bar, BEAM_W * k / 2.0);
            p.shape(ShapeInstance::rrect(
                bar,
                Paint::glow(hex(GREEN, 0.8 * glow), 6.0 * k, 0.6),
            ));
            p.shape(ShapeInstance::rrect(bar, Paint::solid(hex(HOT, glow))));
        }
    }
}

/// The transcript's type: JetBrains Mono, scaled with the popup.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = look.width / CONCEPT_W;
    TextStyle::new(fonts(look).text, TEXT_PX * k, LINE * k)
}

/// Width the transcript wraps at: the screen less its padding and room for
/// the caret.
pub(crate) fn text_width(look: &Look) -> f32 {
    let k = look.width / CONCEPT_W;
    look.width - (2.0 * PAD_X - 6.0) * k
}

impl Popup {
    pub(crate) fn phosphor_surface(look: &Look) -> Size {
        Geo::new(look).surface()
    }

    /// Notes the end without a final text, the caret's moves and revised
    /// words.
    pub(crate) fn sync_phosphor(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Phosphor {
            return;
        }
        if self.ended() && self.extra.final_at.is_none() && self.phos.ended_at.is_none() {
            self.phos.ended_at = Some(t);
        }
        let seen: Vec<(u32, String)> = self.shown.iter().map(|w| (w.id, w.text.clone())).collect();
        // An edit is no revision: no reverse-video flash for it.
        if self.extra.final_at.is_none() && !self.phosphor_editing() {
            for (id, text) in &seen {
                let before = self.phos.seen.iter().find(|(i, _)| i == id);
                let revised = before.is_some_and(|(_, old)| {
                    old != text && !text.starts_with(old.as_str()) && !text.starts_with('…')
                });
                if revised {
                    self.phos.flash = Some((*id, t));
                }
            }
        }
        self.phos.seen = seen;
        let Some(to) = self.phosphor_caret_target() else {
            self.phos.caret = None;
            return;
        };
        match self.phos.caret {
            Some((old, from, moved)) if old != to => {
                // Start the glide from where the caret is now.
                let g = ease_out((t - moved) / CARET_GLIDE);
                let here = Point::new(from.x + (old.x - from.x) * g, from.y + (old.y - from.y) * g);
                self.phos.caret = Some((to, here, t));
            }
            Some(_) => {}
            None => self.phos.caret = Some((to, to, t)),
        }
    }

    /// The shaped transcript and the start of each shown word in its text.
    fn phosphor_shaped(&self) -> (vo::scene::text::ShapedText, Vec<u32>) {
        let style = self.look.text_style();
        let mut text = String::new();
        let mut starts = Vec::with_capacity(self.shown.len());
        for w in &self.shown {
            if !text.is_empty() {
                text.push(' ');
            }
            starts.push(text.len() as u32);
            text.push_str(&w.text);
        }
        let shaped = self
            .measure
            .layout(&text, &style, Some(self.look.text_width()));
        (shaped, starts)
    }

    /// The box of shown word `i` in transcript coordinates.
    fn phosphor_word_box(
        &self,
        shaped: &vo::scene::text::ShapedText,
        starts: &[u32],
        i: usize,
    ) -> Option<Rect> {
        let style = self.look.text_style();
        let w = self.shown.get(i)?;
        let end = starts[i] + w.text.len() as u32;
        let glyphs: Vec<_> = shaped
            .glyphs
            .iter()
            .filter(|g| g.cluster >= starts[i] && g.cluster < end)
            .collect();
        let first = glyphs.first()?;
        let line = (first.pos.y / style.line_height).floor();
        let x0 = glyphs.iter().map(|g| g.pos.x).fold(f32::MAX, f32::min);
        let x1 = x0 + self.measure.layout(&w.text, &style, None).size.w;
        Some(Rect::new(
            x0,
            line * style.line_height,
            x1 - x0,
            style.line_height,
        ))
    }

    /// The text is edited, or was and is sent: the edit caret is the one
    /// caret, until the next recording.
    fn phosphor_editing(&self) -> bool {
        self.edit_view.is_some() || self.edit_sent
    }

    /// Where the caret's top left goes, in screen coordinates: a space after
    /// the last word. None while editing (`PhosphorSite` draws the caret).
    fn phosphor_caret_target(&self) -> Option<Point> {
        if self.phosphor_editing()
            || self.extra.final_at.is_some()
            || self.phos.ended_at.is_some()
            || self.status().finishing && !self.status().loading
        {
            return None;
        }
        let g = Geo::new(&self.look);
        let k = g.k;
        // After the ghost tail when there is one, so the block never covers
        // the first ghost word (s3th). The same tail `signal_marks` lays.
        let tail = model_signals::TailText {
            style: self.look.text_style(),
            width: self.look.text_width(),
            rows: self.lines,
        };
        let (x, y) = if let Some((x, top)) = self.ghost_tail_end(&self.shown, &tail) {
            (x + 8.0 * k, top)
        } else if self.shown.is_empty() {
            (0.0, 0.0)
        } else {
            let (shaped, starts) = self.phosphor_shaped();
            let r = self.phosphor_word_box(&shaped, &starts, self.shown.len() - 1)?;
            (r.x + r.w + 8.0 * k, r.y)
        };
        Some(Point::new(
            PAD_X * k + x,
            TEXT_TOP * k + y + (LINE - 37.0) * k * 0.62,
        ))
    }

    pub(crate) fn phosphor_view(&self) -> Element<Msg> {
        let g = Geo::new(&self.look);
        let size = g.surface();
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        let k = g.k;
        let status = self.status();
        let h = g.screen_h(self.lines);
        let done = self.extra.final_at.is_some();
        let exit = if done {
            Transition::fade().offset(0.0, -8.0)
        } else {
            Transition::fade().scale(0.97)
        };
        // On a failure the border is drawn snapped open (MO-53) instead.
        let broken = self.phosphor_broken(&g, h);
        let screen = at(Element::layer("crt"), MARGIN, MARGIN)
            .size(g.w, h)
            .radius(RADIUS * k)
            .fill_gradient(hex(SCREEN, 0.94), hex(DEEP, 0.96))
            .border(
                hex(GREEN, if broken.is_some() { 0.0 } else { 0.95 }),
                1.8 * k,
            )
            .glow(hex(GREEN, 0.8), 8.0 * k, 0.35)
            // The screen snaps to a new line count, as its canvas rule and the
            // status row do: on a spring it left the status row over its edge.
            .motion(Prop::Size, Motion::Snap)
            .scanlines(0.28, 3.0 * k)
            .crt(0.05)
            .vignette(0.35)
            .flicker(0.025)
            .bloom(0.45, 7.0 * k)
            .presence(Transition::fade().scale(0.97), exit)
            .child(self.phosphor_canvas(&g, &status, h))
            .child(self.phosphor_header(&g, &status))
            .child(self.phosphor_progress(&g, &status))
            .child(self.phosphor_text(&g))
            .child(self.phosphor_status(&g, &status, h))
            .children(broken);
        root.child(screen)
    }

    /// MO-53: the screen's border snapped open on its top edge, in the
    /// border's green with the ends lit pale, or `None` when nothing failed.
    fn phosphor_broken(&self, g: &Geo, h: f32) -> Option<Element<Msg>> {
        if !self.paths_broken() {
            return None;
        }
        let k = g.k;
        let bw = 1.8 * k;
        let border = make::panel(bw / 2.0, bw / 2.0, g.w - bw, h - bw, RADIUS * k);
        let pen = BrokenPen {
            width: bw,
            glow: 2.0,
            line: hex(GREEN, 0.95),
            accent: hex(HOT, 1.0),
            label: TextStyle::new(fonts(&self.look).caption, 11.0 * k, 14.0 * k).tracking(0.6 * k),
            label_y: 8.0 * k,
        };
        // Break on the top edge, a little left of the middle, as type does.
        let len = border.length();
        let top = (0..512)
            .map(|i| len * i as f32 / 512.0)
            .filter_map(|s| border.at(s).map(|(p, _)| (s, p)))
            .filter(|(_, p)| p.y < bw + 0.5)
            .min_by(|a, b| {
                (a.1.x - 0.45 * g.w)
                    .abs()
                    .total_cmp(&(b.1.x - 0.45 * g.w).abs())
            })
            .map_or(0.0, |x| x.0);
        self.paths_broken_frame_at(&border, top, g.w, h, pen)
    }

    /// The transcript as the glyph paths and channels follow it (RD-10):
    /// phosphor's own type and pitch, the pen in its green and pale light.
    fn phosphor_paths_text(&self, g: &Geo) -> PathsText {
        PathsText {
            line: LINE * g.k,
            ink: hex(GREEN, 1.0),
            nib: hex(HOT, 1.0),
            ..PathsText::of(&self.look)
        }
    }

    fn phosphor_canvas(&self, g: &Geo, status: &StatusLine, h: f32) -> Element<Msg> {
        let k = g.k;
        let finishing = status.finishing && !status.loading;
        let sink_at = self
            .extra
            .finish_at
            .or(self.phos.ended_at)
            .or_else(|| finishing.then_some(self.extra.now));
        let flash = self.phos.flash.and_then(|(id, at)| {
            let i = self.shown.iter().position(|w| w.id == id)?;
            let (shaped, starts) = self.phosphor_shaped();
            let r = self.phosphor_word_box(&shaped, &starts, i)?;
            // With the beam, only the letters the revision changed.
            let x = self
                .tally
                .revision(id)
                .filter(|_| self.motion("beam-write"))
                .and_then(|rev| {
                    let b =
                        self.tally_word_box(i, &self.look.text_style(), self.look.text_width())?;
                    b.letters.get(rev.from).map(|l| l.0)
                })
                .unwrap_or(r.x)
                .clamp(r.x, r.x + r.w);
            Some((
                Rect::new(
                    PAD_X * k + x,
                    TEXT_TOP * k + r.y + 4.0 * k,
                    r.x + r.w - x,
                    r.h - 6.0 * k,
                ),
                at,
            ))
        });
        let rule2_y = (TEXT_TOP + self.lines as f32 * LINE + RULE2_GAP * 0.55) * k;
        let screen = Screen {
            k,
            w: g.w,
            shown_at: self.shown_at,
            sink_at,
            loading: status.loading,
            wave: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
            scrub: self.scrub(),
            caret: self.phos.caret,
            flash,
            rec: !status.finishing && !done_or_ended(self),
            no_light: self.extra.final_at.is_some(),
            level: if status.finishing {
                0.0
            } else {
                self.phosphor_level()
            },
            icon: if done_or_ended(self) {
                Icon::None
            } else if status.finishing && !status.loading {
                Icon::Spinner
            } else {
                Icon::Bars
            },
            status_y: h - (BOTTOM_PAD + 4.0) * k,
            rule2_y,
            idle: !self.closing && !status.finishing && !done_or_ended(self) && !self.still(),
            newest: (
                (TEXT_TOP + (self.lines.max(1) - 1) as f32 * LINE) * k,
                (TEXT_TOP + self.lines.max(1) as f32 * LINE) * k,
            ),
            h,
            still: self.still(),
        };
        at(
            Element::canvas("screen", f64::INFINITY, move |t, r, p| {
                p.layer(Affine::translate(r.x, r.y), 1.0, |p| screen.draw(t, p))
            }),
            0.0,
            0.0,
        )
        .size(g.w, h)
    }

    /// The newest level, for the status icon.
    fn phosphor_level(&self) -> f32 {
        self.wave.last().copied().unwrap_or(0.0)
    }

    /// Microphone, title, clock and the REC label.
    fn phosphor_header(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static MIC: OnceLock<Arc<Outline>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let f = fonts(&self.look);
        let style = TextStyle::new(f.caption, LABEL_PX * k, 24.0 * k).tracking(0.6 * k);
        let cy = HEADER_Y * k;
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let mic = MIC.get_or_init(|| Arc::new(Outline::new(&icons::mic(), 0.05)));
        let icon = at(
            Element::stroke_path(
                "mic",
                mic.clone(),
                view,
                StrokeStyle::solid(2.6 * k, hex(GREEN, 1.0)),
            )
            .size(34.0 * k, 34.0 * k)
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
            PAD_X * k - 4.0 * k,
            cy - 18.0 * k,
        );
        let title = at(
            Element::label("title", "VOICE DICTATION")
                .text_style(style)
                .color(hex(GREEN, 1.0)),
            PAD_X * k + 40.0 * k,
            cy - 12.0 * k,
        );
        let clock = format!("00:{}", self.clock);
        let clock_el = at(
            digit_roll(
                "clock",
                &clock,
                style,
                hex(GREEN, 1.0),
                Spring::critical_to_98(0.25),
            ),
            g.w - PAD_X * k - 200.0 * k,
            cy - 12.0 * k,
        );
        let done = self.extra.final_at.is_some();
        let tag = if done {
            "SENT"
        } else if self.phos.ended_at.is_some() {
            "STOP"
        } else if status.loading {
            "LOAD"
        } else if status.finishing {
            "PROC"
        } else {
            "REC"
        };
        let mut layer = at(Element::layer("header"), 0.0, 0.0)
            .size(g.w, RULE_Y * k)
            .child(icon)
            .child(title)
            .child(clock_el)
            .child(
                at(
                    Element::label(format!("tag-{tag}"), tag)
                        .text_style(style)
                        .color(hex(GREEN, 1.0)),
                    g.w - PAD_X * k - 36.0 * k,
                    cy - 11.0 * k,
                )
                .presence(Transition::fade().wipe(1.0), Transition::fade()),
            );
        if done {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            layer = layer.child(at(
                Element::stroke_path(
                    "check",
                    check.clone(),
                    view,
                    StrokeStyle::solid(2.6 * k, hex(HOT, 1.0)),
                )
                .size(18.0 * k, 18.0 * k)
                .motion(Prop::Trim, Spring::critical_to_98(0.3))
                .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                g.w - PAD_X * k - 57.0 * k,
                cy - 8.0 * k,
            ));
        }
        layer
    }

    /// An ASCII progress bar over the sunken meter while a model loads or
    /// the final pass runs.
    fn phosphor_progress(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        let k = g.k;
        let slot = at(Element::layer("progress-slot"), 0.0, 0.0);
        if !self.shows_progress(status) || self.extra.final_at.is_some() {
            return slot;
        }
        let f = fonts(&self.look);
        let fraction = self.files.fraction.map(|f| f.clamp(0.0, 1.0));
        let cells = 30usize;
        let full = fraction.map_or(0, |f| (f * cells as f64).round() as usize);
        let bar: String = std::iter::repeat_n('■', full)
            .chain(std::iter::repeat_n('·', cells - full))
            .collect();
        let pct = fraction.map_or("--".into(), |f| {
            format!("{:>3}%", (f * 100.0).round() as i32)
        });
        let text = format!("[{bar}] {pct}");
        let style = TextStyle::new(f.caption, LABEL_PX * k, 24.0 * k);
        slot.child(at(
            Element::label("progress", text)
                .text_style(style.clamp_lines(1))
                .width(g.w - 2.0 * PAD_X * k)
                .color(hex(GREEN, 1.0))
                .presence(Transition::fade().wipe(1.0), Transition::fade()),
            PAD_X * k,
            (METER_BASE - 46.0) * k,
        ))
    }

    /// The beam's sweeps over the newest words and the newest changed
    /// letters, or `None` under reduced motion and with the pen on.
    /// MO-50: a cancel takes the words back into the caret, drawn over the
    /// hidden text.
    fn phosphor_rewind(&self, style: &TextStyle, size: Size) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let ink = endings::Ink {
            text: hex(GREEN, 0.35),
            mark: hex(HOT, 1.0),
            faint: hex(DIM, 1.0),
        };
        let laid = self.lay_words(*style, self.look.text_width());
        self.rewind_mark("rewind", laid, ink, size)
    }

    fn phosphor_beam(&self, g: &Geo) -> Option<Beam> {
        // The pen writes new words and rewrites changed letters itself
        // (SW-06): a beam on the same letters would write them twice.
        if !self.motion("beam-write") || self.motion("stroke-write") || self.still() {
            return None;
        }
        let k = g.k;
        let style = self.look.text_style();
        let width = self.look.text_width();
        let keep = BEAM_SECS + BEAM_TAIL + 0.3;
        let mut sweeps: Vec<Sweep> = self
            .tally_arrivals(&style, width, keep)
            .into_iter()
            .map(|(b, at)| Sweep {
                line: b.rect,
                x0: b.rect.x,
                x1: b.rect.x + b.rect.w,
                at,
                secs: BEAM_SECS,
            })
            .collect();
        let now = self.extra.now;
        if self.extra.final_at.is_none() {
            for rev in self.tally.revised.iter().filter(|r| now - r.at < keep) {
                let Some(i) = self.shown.iter().position(|w| w.id == rev.id) else {
                    continue;
                };
                let Some(b) = self.tally_word_box(i, &style, width) else {
                    continue;
                };
                let x1 = b.rect.x + b.rect.w;
                let x0 = b.letters.get(rev.from).map_or(x1, |l| l.0);
                if x1 - x0 < 1.0 {
                    continue;
                }
                // The same speed as a whole word, so a short fix is quick.
                let secs = (BEAM_SECS * ((x1 - x0) / b.rect.w.max(1.0)) as f64).max(0.035);
                sweeps.retain(|s| !(s.line == b.rect && s.at > rev.at));
                sweeps.push(Sweep {
                    line: b.rect,
                    x0,
                    x1,
                    at: rev.at,
                    secs,
                });
            }
        }
        Some(Beam {
            sweeps,
            k,
            top: TEXT_TOP * k,
            screen_h: g.screen_h(self.lines),
        })
    }

    /// The transcript, written word by word by the beam.
    fn phosphor_text(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let style = self.look.text_style();
        let mut words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .text_style(style)
            .color(hex(GREEN, 1.0))
            .text_halo(hex(GREEN, 0.35), 5.0 * k)
            .width(self.look.text_width())
            .motion(Prop::Opacity, Spring::critical_to_98(0.08))
            .motion(Prop::Offset, Spring::critical_to_98(0.1))
            .motion(Prop::TextFx, Spring::critical_to_98(0.16));
        // The beam writes new and changed letters itself; without it, as
        // under reduced motion, new words type in with a wipe. A changed
        // word stays as it is: it is already being read, and a wipe from 1
        // would hide it (RD-09).
        let beam = self.phosphor_beam(g);
        let paths = self.phosphor_paths_text(g);
        words = if self.motion("stroke-write") {
            // The pen's fill brings each letter up (SW-02).
            words
                .word_enter(self.ghost_enter(self.paths_word_enter(Transition::NONE)))
                .word_change(Transition::NONE)
        } else if beam.is_some() {
            words
                .word_enter(Transition::NONE)
                .word_change(Transition::NONE)
        } else {
            words
                .word_enter(self.ghost_enter(Transition::NONE.wipe(1.0)))
                .word_change(Transition::NONE)
        };
        if self.phos.ended_at.is_some() {
            words = words.opacity(0.35);
        }
        // MO-24 and the pen's letter fills.
        let words = self.typo_words_in(words, &paths);
        let words = self.handoff_or(
            words,
            &style,
            Some(self.look.text_width()),
            hex(GREEN, 1.0),
            hex(HOT, 1.0),
        );
        let size = Size::new(
            self.look.text_width() + 22.0 * k,
            self.lines as f32 * LINE * k,
        );
        let mut layer = at(Element::layer("text"), PAD_X * k, TEXT_TOP * k)
            .size(size.w, size.h)
            .child(words.rewind_hide(self.rewinding()))
            .children(self.phosphor_rewind(&style, size));
        // The pen over the words while they are being written (MO-80).
        if self.edit_view.is_none() {
            layer = layer.children(self.paths_overlays(&paths));
        }
        if let Some(beam) = beam {
            layer = layer.child(
                at(
                    Element::canvas("beam", beam.until(), move |t, r, p| beam.draw(t, r, p)),
                    0.0,
                    0.0,
                )
                .size(size.w, size.h),
            );
        }
        if self.phos.ended_at.is_some() {
            // Cancelled: a strike line types through the text.
            layer = layer.child(
                at(
                    Element::shape("strike", ShapeKind::RRect),
                    0.0,
                    LINE * k * 0.55,
                )
                .size(self.look.text_width(), 2.0 * k)
                .fill(hex(GREEN, 0.9))
                .presence(Transition::fade().scale(0.0), Transition::fade()),
            );
        }
        // The open seam under a tentative word, in phosphor dots (MO-24).
        let layer = self.typo_marks_in(
            layer,
            &self.shown,
            self.look.text_width(),
            channels::Seam::Dots {
                pitch: 4.0 * k,
                r: 1.1 * k,
                color: hex(GREEN, 1.0),
            },
        );
        self.signal_marks(layer, &self.shown, self.look.text_width(), (0.0, 0.0), hex(GREEN, 1.0))
    }

    /// The italic status line under the lower rule.
    fn phosphor_status(&self, g: &Geo, status: &StatusLine, h: f32) -> Element<Msg> {
        let k = g.k;
        let f = fonts(&self.look);
        let done = self.extra.final_at.is_some();
        let app = self
            .files
            .live
            .as_ref()
            .and_then(|d| d.target.as_ref())
            .map(|t| t.app.clone())
            .filter(|a| !a.is_empty());
        let (state, detail) = if done {
            match app {
                Some(app) => (
                    self.delivery_word().to_string(),
                    format!("into {app} · {} words", self.word_count),
                ),
                None => ("Done".to_string(), format!("{} words", self.word_count)),
            }
        } else if self.phos.ended_at.is_some() {
            ("Cancelled".to_string(), "nothing typed".to_string())
        } else {
            (
                status.state.clone(),
                status
                    .detail
                    .trim()
                    .trim_start_matches('·')
                    .trim()
                    .to_string(),
            )
        };
        let line = if detail.is_empty() {
            state.clone()
        } else {
            format!("{state} · {detail}")
        };
        let style = TextStyle::new(f.status, STATUS_PX * k, 26.0 * k);
        let words: Vec<Word> = line
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| Word {
                // Above the transcript's word ids (bit 24 set), so the status
                // row is never taken for the reading line (RD-09).
                id: 1 << 24 | (state.len() as u32) << 8 | i as u32,
                text: w.to_string(),
            })
            .collect();
        let text = at(
            // Keyed once, not by state: a new key per state faded the old row
            // out under the new one ("Loading" over the hint).
            Element::live_text("status", &words)
                .text_style(style)
                .color(hex(GREEN, 1.0))
                .width(g.w - (2.0 * PAD_X + 50.0) * k)
                .motion(Prop::TextFx, Spring::critical_to_98(0.2))
                .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                .motion(Prop::Offset, Spring::critical_to_98(0.15))
                .word_enter(Transition::NONE.wipe(1.0))
                .word_change(Transition::fade().offset(0.0, 6.0 * k))
                .presence(Transition::NONE.wipe(1.0), Transition::fade()),
            (PAD_X + 50.0) * k,
            h - (BOTTOM_PAD + 4.0 + 13.0) * k,
        )
        // The row snaps to a new line count, as the canvas's rule and level
        // icon do: on a spring it slid down across the rule above it.
        .motion(Prop::Position, Motion::Snap);
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let mark = if done {
            Some((
                "check",
                CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05))),
            ))
        } else if self.phos.ended_at.is_some() {
            Some((
                "cross",
                CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05))),
            ))
        } else {
            None
        };
        let mut layer = at(Element::layer("status"), 0.0, 0.0)
            .size(g.w, h)
            .child(text);
        if let Some((key, outline)) = mark {
            let side = 30.0 * k;
            layer = layer.child(
                at(
                    Element::stroke_path(
                        key,
                        outline.clone(),
                        Rect::new(0.0, 0.0, 24.0, 24.0),
                        StrokeStyle::solid(3.0 * k, hex(GREEN, 1.0)),
                    )
                    .size(side, side)
                    .motion(Prop::Trim, Spring::critical_to_98(0.3))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    PAD_X * k,
                    h - (BOTTOM_PAD + 4.0) * k - side / 2.0,
                )
                .motion(Prop::Position, Motion::Snap),
            );
        }
        layer
    }
}

/// The dictation is over: the final text came or it was cancelled.
fn done_or_ended(p: &Popup) -> bool {
    p.extra.final_at.is_some() || p.phos.ended_at.is_some()
}

/// Phosphor's edit adapter (plan section 4.5): the edit caret in the
/// block caret's green, drawn by the transcript's Flow so it bows with the
/// glyphs. It is a bar, not a block: a block over a letter would hide it,
/// with no reverse video to show it. The beam write is in the quiet list.
pub(crate) struct PhosphorSite;

impl edit_site::EditSite for PhosphorSite {
    fn caret(&self, look: &Look) -> edit_site::CaretStyle {
        let k = Geo::new(look).k;
        edit_site::CaretStyle {
            caret: vo::scene::flow::Caret {
                width: (3.0 * k).max(2.0),
                color: Some(hex(GREEN, 1.0)),
                ..vo::scene::flow::Caret::default()
            },
            selection: hex(GREEN, 0.35),
            mark: hex(GREEN, 1.0),
        }
    }

    /// Only the transcript shows the window: the status row is a live
    /// text too, with words of its own.
    fn place(&self, look: &Look, el: Element<Msg>, view: &edit::EditView) -> Element<Msg> {
        let style = self.caret(look);
        let decor = vo::scene::flow::EditDecor {
            caret_style: style.caret,
            selection_color: Some(style.selection),
            mark_color: Some(style.mark),
            ..view.decor.clone()
        };
        el.map_live_texts(&mut |e| {
            if e.key().to_string() != "words" {
                return e;
            }
            e.live_source(view.text.clone())
                .edit(decor.clone())
                .still_motion(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// s3th: with the ghost tail on, the block caret stands after the last
    /// drawn ghost word, not over the first one.
    #[test]
    fn the_caret_stands_after_the_ghost_tail() {
        use voxtype::osd::live::{ModelSignals, TailToken};
        let fonts = super::super::swiss_look::edit_check::fonts();
        let cfg = OsdConfig {
            theme: OsdTheme::Phosphor,
            motion: OsdMotion::Names(vec![model_signals::GHOST_TAIL.into()]),
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &PlasmaTheme::default(), String::new(), &fonts);
        look.set_motion(&cfg.motion);
        let mut popup = Popup::new(look.clone(), &fonts);
        let committed = "This is a longer test";
        popup.files.recording = true;
        popup.files.preview = committed.into();
        popup.shown = committed
            .split(' ')
            .enumerate()
            .map(|(i, w)| Word {
                id: i as u32 + 1,
                text: w.into(),
            })
            .collect();
        let caret = |p: &Popup| p.phosphor_caret_target().expect("a caret").x;
        let before = caret(&popup);
        let tok = |text: &str, edge| TailToken {
            text: text.into(),
            start: 0.0,
            end: 0.08,
            edge,
            conf: 0.9,
            detail: None,
        };
        popup.model_signals = Some(ModelSignals {
            tail: vec![tok(" of", 5), tok(" voice", 3)],
            with_tail: format!("{committed} of voice"),
            ..ModelSignals::default()
        });
        let after = caret(&popup);
        let style = look.text_style();
        let width = |t: &str| popup.measure.layout(t, &style, None).size.w;
        let want = width(&format!("{committed} of voice")) - width(committed);
        assert!(
            (after - before - want).abs() < 2.0,
            "moved {} for {want}",
            after - before
        );
    }

    fn screen(wave: &[f32]) -> Screen {
        Screen {
            k: 1.0,
            w: 600.0,
            shown_at: 0.0,
            sink_at: None,
            loading: false,
            wave: Arc::from(wave),
            newest_at: 1.0,
            rate: 10.0,
            scrub: endings::Scrub::default(),
            caret: None,
            flash: None,
            rec: true,
            no_light: false,
            level: 0.0,
            icon: Icon::Bars,
            status_y: 0.0,
            rule2_y: 0.0,
            idle: false,
            newest: (0.0, 0.0),
            h: 300.0,
            still: false,
        }
    }

    #[test]
    fn the_middle_column_shows_the_newest_level_in_whole_blocks() {
        let s = screen(&[0.0, 0.0, 0.0, 1.0]);
        let mid = (COLUMNS - 1) / 2;
        assert!(s.blocks(1.0, mid) > 0);
        assert!(s.blocks(1.0, mid) <= BLOCKS);
        assert_eq!(s.blocks(1.0, 0), 0);
    }

    #[test]
    fn the_meter_sinks_to_dots_after_the_stop() {
        let mut s = screen(&[1.0; 40]);
        s.sink_at = Some(1.0);
        let mid = (COLUMNS - 1) / 2;
        assert!(s.blocks(1.0, mid) > 0);
        assert_eq!(s.blocks(1.0 + SINK_SECS, mid), 0);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use super::{hex, GREEN};
    use vo_core::edit::{Dir, EditAction, Unit};
    use voxtype::osd::config::OsdTheme;

    /// M7.L6: Phosphor edits in its own layout, its beam write and
    /// reverse-video flash off. With the caret on the newest, a middle and
    /// the top line, at both widths: the caret on its glyph within 1 px in
    /// every frame, the popup's size unchanged, and the golden
    /// `tests/fixtures/osd-vo/edit-phosphor.txt`.
    #[test]
    fn phosphor_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Phosphor, "phosphor");
    }

    /// M7.L6: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn phosphor_edits_within_the_envelope() {
        envelope(OsdTheme::Phosphor, "phosphor");
    }

    /// The block caret after the last word gives way to the edit caret in
    /// its green, and an edit flashes no word in reverse video.
    #[test]
    fn phosphor_edits_with_one_green_caret_and_no_flash() {
        let mut r = Rig::new(OsdTheme::Phosphor, false);
        r.p.files.recording = true;
        r.dictate(40);
        assert!(r.p.phos.caret.is_some(), "the block caret while listening");
        r.enter(1.0);
        assert!(r.p.phos.caret.is_none(), "no block caret while editing");
        assert_eq!(r.carets(hex(GREEN, 1.0)), 1, "the edit caret");
        // "w39" becomes "w9": a change inside the word, which in a
        // dictation is a revision.
        let back = EditAction::MoveBy {
            dir: Dir::Back,
            unit: Unit::Grapheme,
            extend: false,
        };
        r.act(back, 1.2);
        r.act(EditAction::Delete(Dir::Back, Unit::Grapheme), 1.3);
        assert!(r.buffer_text().ends_with(" w9"), "{}", r.buffer_text());
        assert!(r.p.phos.flash.is_none(), "no reverse-video flash");
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn the_caret_blinks_530_ms_on_and_530_ms_off() {
        assert!(caret_on(-0.1));
        assert!(caret_on(0.0) && caret_on(0.52));
        assert!(!caret_on(0.54) && !caret_on(1.05));
        assert!(caret_on(1.07));
    }

    #[test]
    fn the_scanline_rolls_once_every_5_s() {
        assert!(roll_at(0.0).is_none());
        for k in 0..3 {
            let t0 = ROLL_FIRST + k as f64 * ROLL_EVERY;
            assert!(roll_at(t0 - 0.05).is_none());
            assert!((roll_at(t0 + ROLL_PASS / 2.0).unwrap() - 0.5).abs() < 1e-3);
            assert!(roll_at(t0 + ROLL_PASS + 0.05).is_none());
        }
    }

    // The static stays faint and at most 12 steps a second.
    const _: () = assert!(NOISE_ALPHA <= 0.02 && NOISE_FPS <= 12.0 && ROLL_H == 2.0);
}
