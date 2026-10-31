//! The `dotmatrix` look of the vo popup, after mock-up 21: white dots on
//! black in the manner of a dot-matrix display. The waveform is columns of
//! dots that grow out of a row of dots, the status is set in a dot-matrix
//! face and scrambles into each new state, the transcript is a clean
//! grotesk, and one red dot pulses while recording.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).
//!
//! While the popup is open it keeps a little idle life (feo, RD-02): about
//! 2 % of the meter's unlit dots rise to 12 % ink for 400 ms at random, and
//! below the level floor the meter's dots step one cell right every 2 s and
//! back. Both are drawn by the meter's canvas in steps of 15 a second, far
//! from the transcript; neither runs once the popup closes or under reduced
//! motion.

use super::kit::{at, ease_out, hex, mix, reduced, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};
use vo::scene::path::{Contour, StrokeStyle};
use vo::scene::roll::digit_roll;

/// Colours sampled from the mock-up.
const RED: u32 = 0xFD4A46;
const DOT: u32 = 0xE6E8E6;
const INK: u32 = 0xF2F3F2;
const STATUS: u32 = 0xA9ADAD;
const BODY: u32 = 0x030405;
const EDGE: u32 = 0x2B323E;
const GREY: u32 = 0x5C6066;

/// The panel's width in the mock-up's units; the look scales to the popup.
const DESIGN_W: f32 = 600.0;
const RADIUS: f32 = 18.0;
/// The dot waveform: its box, the columns' pitch and the rows'.
const WAVE_TOP: f32 = 20.0;
const WAVE_H: f32 = 96.0;
const COL_X0: f32 = 28.0;
const COL_X1: f32 = 568.0;
const COL_PITCH: f32 = 9.4;
const ROW_PITCH: f32 = 8.4;
/// Dots above (and below) the baseline at full level.
const ROWS: usize = 5;
const DOT_R: f32 = 2.05;
/// Seconds of audio between neighbouring columns.
const COL_SECS: f64 = 0.03;
/// The transcript: Inter Regular.
const TEXT_X: f32 = 27.0;
const TEXT_PX: f32 = 33.5;
const TEXT_LINE: f32 = 49.6;
const TEXT_TOP: f32 = 7.0;
/// The status row: the red dot, then the dot-matrix words.
const STATUS_TOP: f32 = 43.6;
const STATUS_H: f32 = 20.0;
const STATUS_PX: f32 = 20.0;
const PAD_BOTTOM: f32 = 34.0;
/// The status row ends this far from the panel's right edge.
const STATUS_RIGHT: f32 = 30.0;
const MAX_LINES: u32 = 5;

/// Seconds per letter as the status resolves from scrambled glyphs, and
/// between two scrambles of a letter.
const RESOLVE_SECS: f64 = 0.018;
const SCRAMBLE_SECS: f64 = 0.045;
/// The baseline lights from the centre, this long per column.
const LIGHT_SECS: f64 = 0.006;

/// Dot sortation (MO-60): a new word's dots lift out of the meter, fall
/// into its letters and land; the meter under the word is short of them
/// and regrows them.
const LIFT_SECS: f64 = 0.08;
const FALL_SECS: f64 = 0.2;
const LAND_SECS: f64 = 0.12;
const REGROW_SECS: f64 = 0.22;
/// How far a dot lifts before it falls, and the gap between letters' dots.
const LIFT_PX: f32 = 10.0;
const LETTER_GAP: f64 = 0.015;
/// Letters of one word that take a dot.
const MAX_LETTERS: usize = 12;
/// Rows the meter is short under a new word.
const SHORT_ROWS: usize = 2;
/// The reading-line marks: the empty measure's dots (MO-52) and the seam's
/// under a tentative word (MO-24), pitch and radius.
const MARK_PITCH: f32 = 6.0;
const MARK_R: f32 = 1.3;
const SEAM_PITCH: f32 = 4.0;
const SEAM_R: f32 = 0.9;
/// The word count's room in the status row, and the notch's room.
const NOTCH_ROOM: f32 = 8.0;

/// feo: the idle motion's pace (RD-02's slow-loop cap), the share of
/// unlit dots that twinkle at a time, their ink at the peak, how long one
/// twinkle lasts, and how long the quiet meter holds each side of its drift.
const IDLE_FPS: f64 = 15.0;
const TWINKLE_SHARE: f32 = 0.02;
const TWINKLE_INK: f32 = 0.12;
const TWINKLE_SECS: f64 = 0.4;
const DRIFT_SECS: f64 = 2.0;
/// R2-21-a: each new word strikes the newest columns with an ink bloom
/// that travels left with its audio and fades over `INK_SECS`, leaving
/// grain of alpha `GRAIN_ALPHA` at most.
pub(crate) const TYPEWRITER_INK: &str = "typewriter-ink";
const INK_SECS: f64 = 1.5;
const BLOOM_SECS: f64 = 0.3;
const GRAIN_ALPHA: f32 = 0.1;
/// R2-21-b: the meter becomes a glass ribbon whose thickness follows the
/// level through a critically damped spring of rate `GLASS_RATE`.
pub(crate) const LIQUID_GLASS_FLOW: &str = "liquid-glass-flow";
const GLASS_RATE: f64 = 12.0;
const GLASS_TAPS: usize = 32;
/// R2-21-c: a dot with rings on the meter's line for each of the last
/// `ORBIT_DOTS` words; older ones drift left and fade over `ORBIT_SECS`.
pub(crate) const ORBITAL_WORD_RINGS: &str = "orbital-word-rings";
const ORBIT_DOTS: usize = 8;
const ORBIT_SECS: f64 = 8.0;
const ORBIT_DRIFT: f32 = 6.0;

/// feo: `t` held to the start of its idle step.
fn idle_step(t: f64) -> f64 {
    (t * IDLE_FPS).floor() / IDLE_FPS
}

/// feo: the ink of unlit cell `cell` at `t`, 0 unless it twinkles. Time
/// runs in `TWINKLE_SECS` windows on one clock, so the twinkles change
/// together a few times a second and the frames between them are the same.
/// A cell may light only in every other window (by its parity), at twice
/// `TWINKLE_SHARE`, so it never stays lit for two windows in a row.
fn twinkle(cell: u32, t: f64) -> f32 {
    let window = (t / TWINKLE_SECS).floor() as i64 as u32;
    if (window ^ cell) % 2 == 1
        || noise(cell.wrapping_mul(0x9E37_79B1) ^ window.wrapping_mul(7919)) >= 2.0 * TWINKLE_SHARE
    {
        return 0.0;
    }
    TWINKLE_INK
}

/// feo: how many cells the quiet meter is shifted at `t`: one for every
/// other `DRIFT_SECS`, else none.
fn drift_cells(t: f64) -> f32 {
    if (t / DRIFT_SECS).floor() as i64 % 2 == 1 {
        1.0
    } else {
        0.0
    }
}

/// Seconds from a word's arrival until the meter has regrown its dots.
fn sort_secs() -> f64 {
    LIFT_SECS + FALL_SECS + REGROW_SECS
}

/// How many rows the meter is short `s` seconds after a word took its
/// dots: all of them until they land, then fewer as they regrow.
fn short_rows(s: f64) -> usize {
    let landed = LIFT_SECS + FALL_SECS;
    if !(0.0..sort_secs()).contains(&s) {
        0
    } else if s < landed {
        SHORT_ROWS
    } else {
        let k = 1.0 - (s - landed) / REGROW_SECS;
        (k * SHORT_ROWS as f64).ceil() as usize
    }
}

/// The fonts the look adds to the registry.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    dots: u32,
}

/// The look's fonts, added to `reg` once (`FontRegistry::system`), so
/// every registry, such as a test's, has its own ids.
fn fonts(reg: &FontRegistry) -> Fonts {
    let add = |pattern: &str, or: u32| reg.system(pattern).unwrap_or(or);
    Fonts {
        text: add("Inter", SANS),
        dots: add("Doto:extralight", MONO),
    }
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(0.15)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    look.width - 2.0 * TEXT_X
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

fn panel_h(lines: u32) -> f32 {
    WAVE_TOP + WAVE_H + TEXT_TOP + lines as f32 * TEXT_LINE + STATUS_TOP + STATUS_H + PAD_BOTTOM
}

pub(crate) fn surface(look: &Look) -> Size {
    Size::new(look.width.ceil(), (panel_h(MAX_LINES) + 8.0).ceil())
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// `text` as it shows `since` seconds after it began to resolve: letters up
/// to the front are real, the rest are scrambled glyphs that change.
fn scrambled(text: &str, since: f64) -> String {
    const POOL: &[u8] = b"#%&*+=?@$<>/\\0123456789ABCDEFHKMNRSTXZ";
    if reduced() || since < 0.0 {
        return if since < 0.0 && !reduced() {
            String::new()
        } else {
            text.to_string()
        };
    }
    let front = (since / RESOLVE_SECS) as usize;
    let step = (since / SCRAMBLE_SECS) as u32;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < front || c == ' ' || c == '·' {
                c
            } else if i > front + 10 {
                ' '
            } else {
                let k = noise(step.wrapping_mul(131) ^ i as u32) * POOL.len() as f32;
                POOL[(k as usize).min(POOL.len() - 1)] as char
            }
        })
        .collect()
}

/// The dot waveform.
#[derive(Clone)]
struct Columns {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    progress: Option<f32>,
    /// Spans of the meter that gave their dots to a word, in canvas x, and
    /// when.
    taken: Vec<(f32, f32, f64)>,
    /// feo: the idle twinkle and drift run (open, not reduced).
    idle: bool,
    /// feo: this copy draws only the twinkles, in a canvas of their own, so
    /// the meter's canvas stays the same from frame to frame on room tone.
    twinkles: bool,
    /// R2-21-b: draw the meter as a glass ribbon over its baseline dots.
    glass: bool,
    /// Reduced motion.
    still: bool,
}

impl Columns {
    /// Rows column `x` is short at `t`.
    fn short(&self, x: f32, t: f64) -> usize {
        self.taken
            .iter()
            .filter(|(x0, x1, _)| (*x0..=*x1).contains(&x))
            .map(|(_, _, at)| short_rows(t - at))
            .max()
            .unwrap_or(0)
    }

    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing => f64::INFINITY,
            _ => self.mode_at + 0.6,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = r.w / DESIGN_W;
        let cy = r.y + r.h / 2.0;
        let x0 = r.x + COL_X0 * k;
        let x1 = r.x + COL_X1 * k;
        let n = ((x1 - x0) / (COL_PITCH * k)).floor() as usize + 1;
        let mid = (n - 1) as f32 / 2.0;
        let since = t - self.shown_at;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let drain = match self.mode {
            Mode::Done | Mode::Cancelled => 1.0 - ease_out((t - self.mode_at) / 0.4),
            _ => 1.0,
        };
        // A cancel switches the baseline off from both ends to the centre.
        let off = if self.mode == Mode::Cancelled {
            ease_out((t - self.mode_at) / 0.4) * mid
        } else {
            -1.0
        };
        let head = ((since / 1.2).fract() * n as f64) as f32;
        let idle = self.idle && matches!(self.mode, Mode::Loading | Mode::Listening);
        let step = idle_step(since);
        let drift = if idle && self.mode == Mode::Listening {
            drift_cells(step) * COL_PITCH * k
        } else {
            0.0
        };
        for i in 0..n {
            let from_mid = (i as f32 - mid).abs();
            // The baseline lights from the centre outward as the popup opens.
            if !reduced() && (since as f32) < from_mid * LIGHT_SECS as f32 {
                continue;
            }
            if mid - from_mid < off {
                continue;
            }
            let x = x0 + i as f32 * COL_PITCH * k;
            let dim = match (self.mode, self.progress) {
                (Mode::Finishing, Some(f)) if i as f32 / n as f32 > f => 0.25,
                (Mode::Cancelled, _) => 0.4,
                _ => 1.0,
            };
            if self.mode == Mode::Loading {
                if self.twinkles {
                    if idle {
                        twinkles(i, 0, Point::new(x, cy), k, step, p);
                    }
                    continue;
                }
                // One lit dot runs along the baseline with a short tail.
                let back = (head - i as f32).rem_euclid(n as f32);
                let lit = (1.0 - back / 5.0).max(0.0);
                let c = mix(hex(GREY), hex(DOT), lit);
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.5 + 0.6 * lit,
                    Paint::solid(c),
                ));
                continue;
            }
            // The newest audio is at the right; older columns move left.
            let at = t_eff - (n - 1 - i) as f64 * COL_SECS;
            let jitter = 0.8 + 0.2 * noise((at / COL_SECS).round() as i64 as u32);
            let level = (self.levels.at(at) * jitter).min(1.0) * drain;
            // Quiet columns are the baseline alone, as in the mock-up.
            let rows = (((level - 0.3) / 0.55).clamp(0.0, 1.0) * ROWS as f32).round() as usize;
            // Under a new word the meter is short: top dots gone and the
            // baseline dot dimmed until they regrow.
            let short = self.short(x - r.x, t);
            let rows = rows.saturating_sub(short);
            if self.twinkles {
                if idle {
                    twinkles(i, rows, Point::new(x, cy), k, step, p);
                }
                continue;
            }
            let c = hex(DOT).fade(dim);
            let base_c = if short > 0 { hex(GREY).fade(dim) } else { c };
            let base_r = if rows == 0 { 1.5 } else { DOT_R };
            // Below the level floor the dot drifts a cell and back (feo).
            let base_x = if rows == 0 && short == 0 {
                x + drift
            } else {
                x
            };
            p.shape(ShapeInstance::circle(
                Point::new(base_x, cy),
                base_r * k.min(1.0),
                Paint::solid(base_c),
            ));
            if self.glass {
                continue;
            }
            for row in 1..=rows {
                let u = row as f32 / ROWS as f32;
                let rr = DOT_R * (1.0 - 0.3 * u) * k.min(1.0);
                let dy = row as f32 * ROW_PITCH * k;
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy - dy),
                    rr,
                    Paint::solid(c),
                ));
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy + dy),
                    rr,
                    Paint::solid(c),
                ));
            }
        }
        if self.glass && !self.twinkles && self.mode != Mode::Loading {
            self.draw_glass(t_eff, drain, Rect::new(x0, cy, x1 - x0, r.h / 2.0), k, p);
        }
    }

    /// R2-21-b: the glass ribbon along `band` (its x span, the centre line
    /// as y and the half height as h). Each column's half thickness is the
    /// level through a critically damped spring, so it swells and narrows
    /// like a slow fluid; under reduced motion it is one still band whose
    /// thickness steps with the level.
    fn draw_glass(&self, t: f64, drain: f32, band: Rect, k: f32, p: &mut Painter) {
        let n = 64;
        let reach = band.h * 0.62;
        let half = |at: f64| -> f32 {
            let v = if self.still {
                (self.levels.at(t) * 4.0).round() / 4.0
            } else {
                damped_level(&self.levels, at)
            };
            ((v - 0.2) / 0.55).clamp(0.0, 1.0) * drain
        };
        let mut top = Vec::with_capacity(n + 1);
        let mut bottom = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let u = i as f32 / n as f32;
            let x = band.x + u * band.w;
            // The ends taper to the baseline dots.
            let taper = (std::f32::consts::PI * u).sin().powf(0.6);
            let at = if self.still {
                t
            } else {
                t - (1.0 - u as f64) * n as f64 * COL_SECS * 0.85
            };
            let h = (1.2 + reach * half(at)) * taper;
            let wob = if self.still {
                0.0
            } else {
                0.18 * h * ((u * 9.0) as f64 - t * 1.3).sin() as f32
            };
            top.push(Point::new(x, band.y - h + wob));
            bottom.push(Point::new(x, band.y + h + wob));
        }
        let mut outline = top.clone();
        outline.extend(bottom.iter().rev().copied());
        let glass = StrokeStyle::solid(1.0, Color::WHITE).linear(
            Point::new(0.0, band.y - reach),
            Color::hex(0xA9C8FF, 0.32),
            Point::new(0.0, band.y + reach),
            Color::hex(0x5B7BE8, 0.12),
        );
        p.fill(&[Contour::polyline(&outline, true)], &glass);
        let edge = StrokeStyle::solid(1.0 * k.min(1.0), Color::hex(0xD8E6FF, 0.55)).glow(4.0);
        p.stroke(&Contour::polyline(&top, false), &edge);
        p.stroke(
            &Contour::polyline(&bottom, false),
            &StrokeStyle::solid(0.8, Color::hex(0x9DB4F0, 0.35)),
        );
    }
}

/// R2-21-b: the level at `at` through a critically damped spring of rate
/// `GLASS_RATE`: the level convolved with the spring's step response
/// kernel, `w(s) = r^2 s e^(-r s)`, over the last second.
fn damped_level(levels: &Levels, at: f64) -> f32 {
    let dt = 1.0 / GLASS_TAPS as f64;
    let r = GLASS_RATE;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for j in 0..GLASS_TAPS {
        let s = (j as f64 + 0.5) * dt;
        let w = r * r * s * (-r * s).exp() * dt;
        sum += w * levels.at(at - s) as f64;
        norm += w;
    }
    (sum / norm.max(1e-6)) as f32
}

/// R2-21-a: a strike's bloom and grain alpha `age` seconds after its word:
/// the bloom flashes and is gone after `BLOOM_SECS`, the grain fades to 0
/// at `INK_SECS`. Under reduced motion only a short brightness step.
fn ink_alpha(age: f64, still: bool) -> (f32, f32) {
    if !(0.0..INK_SECS).contains(&age) {
        return (0.0, 0.0);
    }
    if still {
        return (if age < BLOOM_SECS { 0.25 } else { 0.0 }, 0.0);
    }
    let bloom = (1.0 - age / BLOOM_SECS).max(0.0) as f32;
    let grain = GRAIN_ALPHA * (1.0 - (age / INK_SECS) as f32);
    (0.3 * bloom * bloom, grain)
}

/// feo: the twinkling dots of column `i`, whose baseline is at `base`,
/// among its unlit cells (the rows above `lit` on both sides).
fn twinkles(i: usize, lit: usize, base: Point, k: f32, t: f64, p: &mut Painter) {
    for row in lit + 1..=ROWS {
        for side in [-1.0, 1.0] {
            let cell = ((i * ROWS + row) * 2) as u32 + (side > 0.0) as u32;
            let ink = twinkle(cell, t);
            if ink <= 0.0 {
                continue;
            }
            let u = row as f32 / ROWS as f32;
            let rr = DOT_R * (1.0 - 0.3 * u) * k.min(1.0);
            let dy = side * row as f32 * ROW_PITCH * k;
            p.shape(ShapeInstance::circle(
                Point::new(base.x, base.y + dy),
                rr,
                Paint::solid(hex(DOT).fade(ink)),
            ));
        }
    }
}

/// One dot on its way from the meter to a letter.
#[derive(Clone, Copy, Debug)]
struct Flyer {
    from: Point,
    to: Point,
    at: f64,
}

impl Flyer {
    /// Where the dot is `s` seconds after it left, how large and how
    /// bright, or `None` once it has gone into its letter.
    fn at(&self, s: f64) -> Option<(Point, f32, f32)> {
        if s < 0.0 {
            return None;
        }
        if s < LIFT_SECS {
            let k = ease_out(s / LIFT_SECS);
            return Some((Point::new(self.from.x, self.from.y - LIFT_PX * k), 1.0, 1.0));
        }
        let top = Point::new(self.from.x, self.from.y - LIFT_PX);
        let s = s - LIFT_SECS;
        if s < FALL_SECS {
            // Falling: slow off the top, fast into the letter.
            let k = (s / FALL_SECS) as f32;
            let kx = ease_out(s / FALL_SECS);
            let ky = k * k;
            return Some((
                Point::new(
                    top.x + (self.to.x - top.x) * kx,
                    top.y + (self.to.y - top.y) * ky,
                ),
                1.0,
                1.0,
            ));
        }
        let s = s - FALL_SECS;
        if s < LAND_SECS {
            // It lands bright and sinks into the ink.
            let k = (s / LAND_SECS) as f32;
            return Some((self.to, 1.0 - 0.6 * k, 1.0 - k));
        }
        None
    }
}

/// The dots falling from the meter into new words, over the whole panel.
#[derive(Clone)]
struct Sort {
    flyers: Vec<Flyer>,
}

impl Sort {
    fn until(&self) -> f64 {
        self.flyers
            .iter()
            .map(|f| f.at + LIFT_SECS + FALL_SECS + LAND_SECS)
            .fold(0.0, f64::max)
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        for f in &self.flyers {
            let Some((q, size, a)) = f.at(t - f.at) else {
                continue;
            };
            let q = Point::new(r.x + q.x, r.y + q.y);
            p.shape(ShapeInstance::circle(
                q,
                DOT_R * 1.6 * size,
                Paint::glow(hex(DOT).fade(0.35 * a), 4.0, 0.5),
            ));
            p.shape(ShapeInstance::circle(
                q,
                DOT_R * size,
                Paint::solid(hex(DOT).fade(a)),
            ));
        }
    }
}

/// The status mark: a red dot that pulses, a ring of dots that turns, a
/// white dot, or a grey one.
#[derive(Clone)]
struct Mark {
    mode: Mode,
    mode_at: f64,
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
        let s = r.w.min(r.h) / 2.0;
        match self.mode {
            Mode::Loading | Mode::Listening => {
                let beat = if reduced() {
                    1.0
                } else {
                    0.5 + 0.5 * (t * std::f64::consts::TAU).cos() as f32
                };
                p.shape(ShapeInstance::circle(
                    c,
                    s * 1.1,
                    Paint::glow(hex(RED).fade(0.35 * beat), 6.0, 0.6),
                ));
                p.shape(ShapeInstance::circle(
                    c,
                    s * (0.85 + 0.15 * beat),
                    Paint::solid(hex(RED)),
                ));
            }
            Mode::Finishing => {
                let lit = ((t / 0.1) as i64).rem_euclid(8) as usize;
                for i in 0..8 {
                    let a = i as f32 / 8.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let q = Point::new(c.x + a.cos() * s * 0.8, c.y + a.sin() * s * 0.8);
                    let on = if i == lit { 1.0 } else { 0.3 };
                    p.shape(ShapeInstance::circle(
                        q,
                        1.6,
                        Paint::solid(hex(DOT).fade(on)),
                    ));
                }
            }
            Mode::Done => {
                let k = ease_out((t - self.mode_at) / 0.25);
                p.shape(ShapeInstance::circle(
                    c,
                    s * 0.9 * k.max(0.3),
                    Paint::solid(hex(DOT)),
                ));
            }
            Mode::Cancelled => {
                p.shape(ShapeInstance::circle(c, s * 0.85, Paint::solid(hex(GREY))));
            }
        }
    }
}

impl Popup {
    pub(crate) fn dotmatrix_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.dotmatrix_panel())
    }

    pub(crate) fn dotmatrix_panel(&self) -> Element<Msg> {
        let l = &self.look;
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let (sort, taken) = self.dotmatrix_sort();
        let columns = Columns {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
            progress: self.kit_progress(),
            taken,
            idle: !self.closing && !reduced() && !l.on("reduced-motion"),
            twinkles: false,
            glass: l.on(LIQUID_GLASS_FLOW),
            still: self.still(),
        };
        let until = columns.until();
        let twinkle = columns.idle.then(|| {
            let twinkle = Columns {
                twinkles: true,
                ..columns.clone()
            };
            at(
                Element::canvas("twinkle", until, move |t, r, p| twinkle.draw(t, r, p)),
                0.0,
                WAVE_TOP,
            )
            .size(l.width, WAVE_H)
        });
        let wave = Element::canvas("dots", until, move |t, r, p| columns.draw(t, r, p))
            .size(l.width, WAVE_H)
            .margin_edges(WAVE_TOP, 0.0, 0.0, 0.0);
        Element::panel("panel")
            .width(l.width)
            .pad_edges(0.0, 0.0, PAD_BOTTOM, 0.0)
            .radius(RADIUS)
            .fill(hex(BODY).fade(0.97))
            .border(hex(EDGE), 1.0)
            .presence(Transition::fade().offset(0.0, 6.0), exit)
            .child(self.dotmatrix_cast())
            .children(self.limit_line(
                self.dotmatrix_text_origin(),
                Size::new(l.width, panel_h(self.lines.min(MAX_LINES))),
                hex(DOT).fade(0.5),
                hex(RED),
            ))
            .child(wave)
            .children(twinkle)
            .children(self.dotmatrix_ink(mode))
            .children(self.dotmatrix_orbits(mode))
            .child(self.dotmatrix_text(mode))
            .child(self.dotmatrix_status(mode, mode_at))
            .child(
                at(
                    Element::canvas("sort", sort.until(), move |t, r, p| sort.draw(t, r, p)),
                    0.0,
                    0.0,
                )
                .size(l.width, panel_h(self.lines.min(MAX_LINES))),
            )
    }

    /// The words on screen that came while listening, newest last, with
    /// when each came and the level then.
    fn dotmatrix_arrivals(&self, n: usize) -> Vec<(f64, f32)> {
        let levels = self.kit_raw_levels();
        let mut out: Vec<(f64, f32)> = self
            .shown
            .iter()
            .filter_map(|w| self.heard.word(w.id).map(|h| h.born))
            .map(|born| (born, levels.at(born)))
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        let skip = out.len().saturating_sub(n);
        out.split_off(skip)
    }

    /// R2-21-a: each new word strikes the newest columns: an ink bloom on
    /// their dots that travels left with its audio, then faint grain that
    /// fades over `INK_SECS`.
    fn dotmatrix_ink(&self, mode: Mode) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(TYPEWRITER_INK) || !matches!(mode, Mode::Listening | Mode::Finishing) {
            return None;
        }
        let now = self.extra.now;
        let strikes: Vec<(f64, f32)> = self
            .dotmatrix_arrivals(16)
            .into_iter()
            .filter(|(born, _)| now - born < INK_SECS)
            .collect();
        if strikes.is_empty() {
            return None;
        }
        let until = strikes
            .iter()
            .map(|s| s.0 + INK_SECS)
            .fold(f64::NEG_INFINITY, f64::max);
        let still = self.still();
        let canvas = Element::canvas("ink", until, move |t, r, p| {
            let k = r.w / DESIGN_W;
            let cy = r.y + r.h / 2.0;
            let right = r.x + COL_X1 * k;
            for &(born, level) in &strikes {
                let age = t - born;
                let (bloom, grain) = ink_alpha(age, still);
                if bloom <= 0.0 && grain <= 0.0 {
                    continue;
                }
                // The strike rides with its audio, one column per COL_SECS.
                let x = if still {
                    right
                } else {
                    right - (age / COL_SECS) as f32 * COL_PITCH * k
                };
                if x < r.x + COL_X0 * k {
                    continue;
                }
                let reach = (0.35 + 0.65 * (level * 1.8).min(1.0)) * r.h * 0.42;
                if bloom > 0.0 {
                    p.shape(ShapeInstance::rrect(
                        RRect::new(
                            Rect::new(x - 3.0 * k, cy - reach, 6.0 * k, 2.0 * reach),
                            3.0 * k,
                        ),
                        Paint::glow(hex(INK).fade(bloom), 4.0 * k, 0.4),
                    ));
                }
                let seed = (born * 1000.0) as u32;
                for g in 0..14u32 {
                    let gx = x + (noise(seed + g * 3) - 0.5) * 3.0 * COL_PITCH * k;
                    let gy = cy + (noise(seed + g * 3 + 1) - 0.5) * 2.0 * reach;
                    let a = grain * (0.4 + 0.6 * noise(seed + g * 3 + 2));
                    p.shape(ShapeInstance::circle(
                        Point::new(gx, gy),
                        0.7 * k.min(1.0),
                        Paint::solid(hex(INK).fade(a)),
                    ));
                }
            }
        });
        Some(at(canvas, 0.0, WAVE_TOP).size(l.width, WAVE_H))
    }

    /// R2-21-c: a dot with rings on the meter's line for each of the last
    /// `ORBIT_DOTS` words, the rings as wide as the word was loud. The
    /// newest glows; older ones drift left and fade. Under reduced motion
    /// still dots without rings.
    fn dotmatrix_orbits(&self, mode: Mode) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(ORBITAL_WORD_RINGS) || !matches!(mode, Mode::Listening | Mode::Finishing) {
            return None;
        }
        let words = self.dotmatrix_arrivals(ORBIT_DOTS);
        if words.is_empty() {
            return None;
        }
        let still = self.still();
        let live = mode == Mode::Listening;
        let until = if still || !live {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        let canvas = Element::canvas("orbits", until, move |t, r, p| {
            let k = r.w / DESIGN_W;
            let cy = r.y + r.h / 2.0;
            let (x0, x1) = (r.x + COL_X0 * k, r.x + COL_X1 * k);
            let n = words.len();
            for (i, &(born, level)) in words.iter().enumerate() {
                let newest = i + 1 == n;
                let age = (t - born).max(0.0);
                let fade = (1.0 - age / ORBIT_SECS).max(0.15) as f32;
                // Each word sits a slot left of the next; the slots drift
                // left as the words age.
                let back = (n - 1 - i) as f32;
                let x = if still {
                    x1 - (back + 0.5) * (x1 - x0) / ORBIT_DOTS as f32
                } else {
                    x1 - 18.0 * k - back * 52.0 * k - ORBIT_DRIFT * k * age as f32
                };
                if x < x0 {
                    continue;
                }
                let c = Point::new(x, cy);
                let core = if newest { 3.2 } else { 2.2 } * k.min(1.0);
                if newest && !still {
                    let glow = (1.0 - age / 1.2).max(0.35) as f32;
                    p.shape(ShapeInstance::circle(
                        c,
                        core * 3.0,
                        Paint::glow(Color::hex(0xF4D9A0, 0.5 * glow), 8.0 * k, 0.6),
                    ));
                }
                p.shape(ShapeInstance::circle(
                    c,
                    core,
                    Paint::solid(Color::hex(0xF7E4BC, 0.9 * fade)),
                ));
                if still {
                    continue;
                }
                let rings = 1 + (level * 4.0).min(2.0) as u32;
                let reach = 8.0 + 22.0 * (level * 1.8).min(1.0);
                for j in 0..rings {
                    let grow = ease_out(age / 0.6);
                    let rad = (reach * (j + 1) as f32 / rings as f32 + 2.0 * age.min(3.0) as f32)
                        * grow
                        * k.min(1.0);
                    let mut ring = ShapeInstance::circle(c, rad.max(0.5), Paint::NONE);
                    ring.stroke = Paint::solid(Color::hex(0xE9C98E, 0.32 * fade / (j + 1) as f32));
                    ring.stroke_width = 0.8;
                    p.shape(ring);
                }
            }
        });
        Some(at(canvas, 0.0, WAVE_TOP).size(l.width, WAVE_H))
    }

    /// The transcript's top left in the panel.
    fn dotmatrix_text_origin(&self) -> Point {
        Point::new(TEXT_X, WAVE_TOP + WAVE_H + TEXT_TOP)
    }

    /// The dots on their way into the new words, in panel coordinates, and
    /// the spans of the meter they left, in the meter canvas's x (MO-60).
    fn dotmatrix_sort(&self) -> (Sort, Vec<(f32, f32, f64)>) {
        if !self.motion("dot-sort") || self.still() || self.extra.final_at.is_some() {
            return (Sort { flyers: Vec::new() }, Vec::new());
        }
        let l = &self.look;
        let style = l.text_style();
        let o = self.dotmatrix_text_origin();
        let k = l.width / DESIGN_W;
        let (x0, x1) = (COL_X0 * k, COL_X1 * k);
        let pitch = COL_PITCH * k;
        let cy = WAVE_TOP + WAVE_H / 2.0;
        let mut flyers = Vec::new();
        let mut taken = Vec::new();
        for (b, at) in self.tally_arrivals(&style, l.text_width(), sort_secs()) {
            let n = b.letters.len().min(MAX_LETTERS);
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for (i, &(lx, lw)) in b.letters.iter().take(n).enumerate() {
                let to = Point::new(o.x + lx + lw / 2.0, o.y + b.baseline - style.size * 0.35);
                // The meter column over the letter gives the dot.
                let col = ((to.x - x0) / pitch).round().max(0.0);
                let fx = (x0 + col * pitch).min(x1);
                lo = lo.min(fx);
                hi = hi.max(fx);
                flyers.push(Flyer {
                    from: Point::new(fx, cy - ROW_PITCH * k),
                    to,
                    at: at + i as f64 * LETTER_GAP,
                });
            }
            if lo <= hi {
                taken.push((lo - pitch / 2.0, hi + pitch / 2.0, at));
            }
        }
        (Sort { flyers }, taken)
    }

    /// The new word's cast into the notch by the word count (MO-65), as a
    /// block of dots under the transcript.
    fn dotmatrix_cast(&self) -> Element<Msg> {
        let l = &self.look;
        if !self.motion("count-cast") {
            return at(Element::layer("cast"), 0.0, 0.0);
        }
        let (count_x, status_y) = self.dotmatrix_count_at();
        let notch = Rect::new(
            count_x - NOTCH_ROOM + 1.0,
            status_y + 3.0,
            2.5,
            STATUS_H - 6.0,
        );
        let casts = self.tally_casts(self.dotmatrix_text_origin(), notch, hex(DOT), true);
        at(
            Element::canvas("cast", casts.until(), move |t, r, p| casts.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(l.width, panel_h(self.lines.min(MAX_LINES)))
    }

    /// The word count's left edge and the status row's top, in the panel.
    fn dotmatrix_count_at(&self) -> (f32, f32) {
        let l = &self.look;
        let style = TextStyle::new(fonts(&l.fonts).dots, STATUS_PX, STATUS_H + 4.0);
        let w = self.measure.layout("0", &style, None).size.w * self.tally_count().len() as f32
            + self.measure.layout(" WORDS", &style, None).size.w;
        let y = WAVE_TOP
            + WAVE_H
            + TEXT_TOP
            + self.lines.min(MAX_LINES) as f32 * TEXT_LINE
            + STATUS_TOP;
        (l.width - STATUS_RIGHT - w, y)
    }

    fn dotmatrix_text(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        // A new word rises into place. With the dots (MO-60) it starts at
        // 70 % ink, readable at once, as its dots fall into it. The fade is
        // short either way, so a word that grows while it fades in (its new
        // letters fade in too), or wraps to the next line, keeps the
        // reading line's ink.
        let plain = !self.motion("dot-sort") || self.still();
        let enter = Transition::fade()
            .opacity(if plain { 0.0 } else { 0.7 })
            .offset(0.0, 6.0);
        let fade = 0.12;
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .text_style(l.text_style())
            .color(hex(INK))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(fade))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(enter));
        // Tentative words at lower ink (MO-24).
        let words = self.typo_words(words);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            hex(INK),
            hex(RED),
        );
        let ink = endings::Ink {
            text: hex(INK),
            mark: hex(DOT),
            faint: hex(GREY),
        };
        let mut text = Element::column("text")
            .height(self.lines.min(MAX_LINES) as f32 * TEXT_LINE)
            .child(words)
            // The empty measure (MO-52), in dots.
            .children(self.line_marks_with(
                l.text_style(),
                l.text_width(),
                ink,
                endings::Cancel::Unthread,
                endings::Pen::Dots {
                    pitch: MARK_PITCH,
                    r: MARK_R,
                },
            ));
        if self.text_above() && !self.shown.is_empty() && self.delivered().is_none() {
            text = text.child(self.dotmatrix_ellipsis(dim));
        }
        // The open seam under a tentative word, in dots (MO-24).
        let text = self.typo_marks_in(
            text,
            &self.shown,
            l.text_width(),
            channels::Seam::Dots {
                pitch: SEAM_PITCH,
                r: SEAM_R,
                color: hex(DOT),
            },
        );
        let text = self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), hex(INK));
        // The text area clips to its own box, as in Popup::text_area, so a line that leaves
        // as the text scrolls slides behind the top edge instead of over the waveform. The
        // box reaches into the left margin for the "…" and into the gap above the status row
        // for descenders; the negative margins keep the layout as it was.
        use vo::scene::taffy::LengthPercentage;
        let (left, below) = (TEXT_X - 2.0, STATUS_TOP - 4.0);
        Element::layer("textclip")
            .clip()
            .margin_edges(TEXT_TOP, TEXT_X, -below, TEXT_X - left)
            .layout(move |s| {
                s.padding.left = LengthPercentage::length(left);
                s.padding.bottom = LengthPercentage::length(below);
            })
            .child(text)
    }

    /// The "…" before cut text, in the left margin so it never changes where the lines
    /// break. Smaller than the text when the margin is narrow.
    fn dotmatrix_ellipsis(&self, dim: f32) -> Element<Msg> {
        use vo::scene::taffy::{LengthPercentageAuto, Position, Rect};
        let style = self.look.text_style();
        let w = self.measure.layout("…", &style, None).size.w;
        let room = TEXT_X - 6.0;
        let mut style = style;
        if w > room {
            style.size *= room / w;
        }
        let w = w.min(room);
        Element::label("ellipsis", "…")
            .text_style(style)
            .color(hex(INK))
            .opacity(0.55 * dim)
            .layout(move |s| {
                s.position = Position::Absolute;
                s.inset = Rect {
                    left: LengthPercentageAuto::length(-w - 2.0),
                    top: LengthPercentageAuto::length(0.0),
                    right: LengthPercentageAuto::auto(),
                    bottom: LengthPercentageAuto::auto(),
                };
            })
    }

    /// The status row's words in `mode`.
    fn dotmatrix_status_text(&self, mode: Mode) -> String {
        let status = self.status();
        match mode {
            Mode::Done => match self.kit_target() {
                Some(app) => format!("{} · {app}", self.delivery_word()),
                None => self.delivery_word().to_string(),
            },
            // No speech and a failure say so; a cancel is the default.
            Mode::Cancelled => match self.ending() {
                Some(Ending::Empty) => Ending::Empty.label().to_string(),
                Some(Ending::Failed) => "Failed · nothing was typed".to_string(),
                _ => "Cancelled · nothing was typed".to_string(),
            },
            _ => format!("{}{}", status.state, status.detail.replace("  ·  ", " · ")),
        }
    }

    /// When the status label began to resolve: the label scrambles in when
    /// the state changes; the detail of the same state (a number counting
    /// down) changes in place.
    fn dotmatrix_status_from(&self, mode_at: f64) -> f64 {
        mode_at.max(self.shown_at + 0.1)
    }

    /// Keeps the tick armed while the status label resolves. It changes
    /// with the time alone, so after the last event (the end of a
    /// dictation with no speech) nothing else would draw it.
    pub(crate) fn arm_dotmatrix_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.look.theme != OsdTheme::Dotmatrix
            || self.extra.tick.is_some()
            || self.closing
            || reduced()
        {
            return;
        }
        let mode = self.kit_mode();
        let from = self.dotmatrix_status_from(self.kit_mode_at(mode));
        let len = self.dotmatrix_status_text(mode).chars().count();
        if cx.t() < from + len as f64 * RESOLVE_SECS {
            self.extra.tick = Some(cx.after(SCRAMBLE_SECS / 2.0, Msg::Tick));
        }
    }

    fn dotmatrix_status(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(fonts(&l.fonts).dots, STATUS_PX, STATUS_H + 4.0);
        let text = self.dotmatrix_status_text(mode);
        let since = self.extra.now - self.dotmatrix_status_from(mode_at);
        let shown = scrambled(&text, since);
        let mark = Mark { mode, mode_at };
        let until = mark.until();
        // The count keeps its width; a long state label is cut short instead.
        let count = Element::row("count")
            .align_items(AlignItems::CENTER)
            .shrink(0.0)
            .child(digit_roll(
                "count",
                &self.tally_count(),
                style,
                hex(STATUS),
                Spring::critical_to_98(0.28),
            ))
            .child(
                Element::label("words", " WORDS")
                    .text_style(style)
                    .color(hex(STATUS)),
            );
        let row = Element::row("status")
            .width(l.width - 30.0 - STATUS_RIGHT)
            .height(STATUS_H)
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_TOP, 0.0, 0.0, 30.0)
            .child(
                Element::canvas("mark", until, move |t, r, p| mark.draw(t, r, p)).size(17.0, 17.0),
            )
            .child(
                Element::label("state", shown)
                    .text_style(style.clamp_lines(1))
                    .color(hex(STATUS))
                    .margin_edges(0.0, 0.0, 0.0, 12.0),
            )
            .child(Element::row("spacer").grow(1.0));
        // The word count, the notch its casts land in (MO-65).
        if self.motion("count-cast") {
            row.child(count)
        } else {
            row
        }
    }
}

#[cfg(test)]
mod font_tests {
    use super::super::*;

    /// A look's font ids belong to the registry it draws with: tests that
    /// each load their own registry must not draw with another's ids.
    #[test]
    fn each_registry_has_its_own_font_ids() {
        let plasma = PlasmaTheme::default();
        for theme in [OsdTheme::Dotmatrix, OsdTheme::Swissfi, OsdTheme::Amber] {
            let cfg = OsdConfig {
                theme,
                ..OsdConfig::default()
            };
            let a = load_fonts(&plasma).unwrap();
            let b = load_fonts(&plasma).unwrap();
            // One more face in b moves the ids the look adds there.
            b.add_system("monospace").unwrap();
            let font = |fonts: &FontRegistry| {
                Look::new(&cfg, &plasma, String::new(), fonts)
                    .text_style()
                    .font
            };
            let (fa, fb) = (font(&a), font(&b));
            if fa == SANS {
                continue; // the look's font is not installed here
            }
            assert_eq!(fb, fa + 1, "{theme:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::moments::{scenario, Moment};
    use super::super::swiss_look::edit_check::fonts as test_fonts;
    use super::*;
    use vo::scene::display::{DisplayList, ItemKind};
    use vo::scene::scene::Scene;
    use vo::scene::text::ShapedLayout;

    /// The dot-face glyphs in `dl`, left to right.
    fn dots(dl: &DisplayList, font: u32) -> Vec<u32> {
        let mut gs: Vec<(f32, u32)> = Vec::new();
        for (i, l) in dl.layers.iter().enumerate() {
            for it in dl
                .layer_items(i as _)
                .iter()
                .filter(|it| it.kind == ItemKind::Glyphs)
            {
                for g in &dl.glyphs[it.start as usize..(it.start + it.len) as usize] {
                    if g.font == font && g.color.a > 0.05 {
                        gs.push((l.transform.apply(g.pos).x, g.glyph));
                    }
                }
            }
        }
        gs.sort_by(|a, b| a.0.total_cmp(&b.0));
        gs.into_iter().map(|g| g.1).collect()
    }

    /// wucu: the status label scrambles in by the clock alone, so after a
    /// cancel, the last event, it froze half resolved ("Cancelled ·
    /// +7E=<H<"). It now asks for frames until it reads in full, at both
    /// widths. No speech says so rather than "Cancelled".
    #[test]
    fn dotmatrix_status_resolves_after_the_ending() {
        let reg = test_fonts();
        let plasma = PlasmaTheme::default();
        // A cut label ends in "…", so only its start is checked.
        for (moment, label) in [
            (Moment::Cancel, "Cancelled · noth"),
            (Moment::NoSpeech, "No speech"),
            (Moment::Failure, "Failed · noth"),
        ] {
            for width_px in [400, 600] {
                let cfg = OsdConfig {
                    theme: OsdTheme::Dotmatrix,
                    width_px,
                    ..OsdConfig::default()
                };
                let mut look = Look::new(&cfg, &plasma, String::new(), &reg);
                look.set_motion(&cfg.motion);
                let dot = fonts(&reg).dots;
                let style = TextStyle::new(dot, STATUS_PX, STATUS_H + 4.0);
                let mut scene: Scene<Msg> =
                    Scene::new().with_text(Box::new(ShapedLayout::new(reg.clone())));
                scene.set_viewport(0.0, Size::new(600.0, 100.0), 1.0);
                scene.apply(Element::label("state", label).text_style(style), 0.0);
                let mut want = DisplayList::default();
                scene.sample_into(0.0, &mut want);
                let want = dots(&want, dot);
                assert!(want.len() >= label.replace([' ', '·'], "").chars().count());

                let options = Options {
                    fonts: reg.clone(),
                    default_motion: default_motion(&look),
                    ..Options::default()
                };
                let mut last = Vec::new();
                vo::envelope::sample_scenario(
                    Popup::new(look.clone(), &reg),
                    &scenario(&look, moment),
                    10.0,
                    options,
                    // The last frame with the label in it: as the popup
                    // fades out, only the count's digit roll is left.
                    &mut |_, _, dl| {
                        let got = dots(dl, dot);
                        if got.len() >= want.len() {
                            last = got;
                        }
                    },
                );
                assert!(
                    last.windows(want.len()).any(|w| w == want.as_slice()),
                    "{moment:?} at {width_px} px: the last status reads {last:?}, not {label:?} {want:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn about_2_percent_of_unlit_dots_twinkle_up_to_12_percent_ink() {
        let (cells, steps) = (600u32, 150);
        let (mut lit, mut peak) = (0usize, 0.0f32);
        for s in 0..steps {
            let t = idle_step(10.0 + s as f64 / IDLE_FPS);
            for c in 0..cells {
                let ink = twinkle(c, t);
                assert!((0.0..=TWINKLE_INK).contains(&ink), "{ink}");
                lit += (ink > 0.0) as usize;
                peak = peak.max(ink);
            }
        }
        let share = lit as f32 / (cells as usize * steps) as f32;
        assert!((0.01..0.03).contains(&share), "share {share}");
        assert!(peak > 0.9 * TWINKLE_INK, "peak {peak}");
    }

    #[test]
    fn a_twinkle_lasts_one_window_at_most() {
        for c in 0..600u32 {
            let mut run = 0;
            for s in 0..300 {
                if twinkle(c, s as f64 / IDLE_FPS) > 0.0 {
                    run += 1;
                    assert!(run as f64 <= TWINKLE_SECS * IDLE_FPS + 1.0, "cell {c}");
                } else {
                    run = 0;
                }
            }
        }
    }

    #[test]
    fn the_twinkles_change_together_once_a_window() {
        for c in 0..600u32 {
            for w in 0..20 {
                let t0 = w as f64 * TWINKLE_SECS;
                let ink = twinkle(c, t0 + 0.01);
                for s in 1..6 {
                    assert_eq!(twinkle(c, t0 + 0.01 + s as f64 * 0.06), ink, "cell {c}");
                }
            }
        }
    }

    #[test]
    fn the_quiet_meter_drifts_one_cell_every_2_s_and_back() {
        assert_eq!(drift_cells(0.5), 0.0);
        assert_eq!(drift_cells(2.5), 1.0);
        assert_eq!(drift_cells(4.5), 0.0);
        assert_eq!(drift_cells(6.5), 1.0);
        assert_eq!(idle_step(1.0 / IDLE_FPS * 3.5), 3.0 / IDLE_FPS);
    }
}

#[cfg(test)]
mod r2_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_ink_grain_stays_faint_and_is_gone_after_1_5_s() {
        for still in [false, true] {
            for i in 0..=200 {
                let age = i as f64 * 0.01;
                let (bloom, grain) = ink_alpha(age, still);
                assert!(
                    bloom <= 0.3 && grain <= GRAIN_ALPHA,
                    "{age}: {bloom} {grain}"
                );
                if age >= BLOOM_SECS {
                    assert_eq!(bloom, 0.0, "{age}");
                }
                if age >= INK_SECS {
                    assert_eq!((bloom, grain), (0.0, 0.0), "{age}");
                }
            }
        }
        assert_eq!(ink_alpha(0.5, true).1, 0.0);
    }

    #[test]
    fn the_glass_level_stays_in_range_and_settles() {
        let n = 3 * FRAME_HZ as usize;
        let step: Vec<f32> = (0..n)
            .map(|i| if i < FRAME_HZ as usize { 0.0 } else { 0.8 })
            .collect();
        let levels = Levels {
            levels: Arc::from(step),
            newest_at: 3.0,
        };
        let mut prev = 0.0;
        for i in 0..=60 {
            let v = damped_level(&levels, 1.0 + i as f64 / 30.0);
            assert!((0.0..=0.8 + 1e-4).contains(&v), "{v}");
            // A critically damped spring never overshoots: it only rises.
            assert!(v >= prev - 1e-4, "{v} after {prev}");
            prev = v;
        }
        assert!(prev > 0.79, "settled at {prev}");
    }
}
