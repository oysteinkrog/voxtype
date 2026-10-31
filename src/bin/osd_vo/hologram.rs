//! The `hologram` look of the vo popup, after concept 20, floating hologram
//! text: no panel, only light. A glowing ring pedestal sits at the bottom
//! with the status on it, a faint column of projected light rises from it
//! with fine scan lines, the waveform stands at its base, the words float
//! in the column in translucent blue type with a soft glow, and a glass
//! microphone orb hangs at the top.
//!
//! The words start under the orb; the wave and the ring sit below the last
//! line, so the projection grows down as lines come.
//! While the final text is made the wave lowers and the ring says so; a
//! cancel greys the light and dims the words. Under reduced motion the scan
//! lines and the ring's shimmer stand still.
//!
//! Round 2 (R2-20), each opt-in by its motion feature:
//!
//! - `typewriter-waves`: a calm pool under the newest line holds a faint
//!   reflection of its words, short streaks that waver for 3 s after a word,
//!   and each word sends one ring at its place, gone in 1.2 s (alpha 20 % or
//!   less). Reduced motion: a still reflection and no rings.
//! - `orbiting-words`: three thin elliptical orbits round the text with a
//!   satellite dot each, their pace the voice summed over time; a new word's
//!   faint copy (25 % alpha or less) runs along the nearest orbit, turned
//!   with it, and lands on the word, gone 500 ms after the word came. The
//!   word itself arrives at 70 % ink on its baseline. Reduced motion: still
//!   orbits, no copy.
//! - `breathing-scroll`: a translucent scroll behind the text, curled at
//!   its top and foot, grows with each line and fades into the app once the
//!   text is done. Its sides bow a little with the voice. Reduced motion: a
//!   flat strip.

use super::glyph_paths::PathsText;
use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use std::collections::HashMap;
use vo::scene::canvas::Painter;
use vo::scene::depth_stack::{DepthStack, Place, Sheet};
use vo::scene::display::{GlyphInstance, GlyphXform, NO_WORD};
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect, Rect};
use vo::scene::glyph_morph::WordOutline;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo_core::words::WordId;

/// Colours sampled from the redesign.
const ICE: u32 = 0xdcecff;
const TEXT: u32 = 0xc6dcff;
const HALO: u32 = 0x3d7bff;
const CYAN: u32 = 0x3ad2ff;
const BLUE: u32 = 0x3d6bff;
const VIOLET: u32 = 0xa064ff;
const ORB: u32 = 0x0b1636;
const HINT: u32 = 0xa9c2f0;
const DIM: u32 = 0x67708a;
const GREEN: u32 = 0x39d98a;

/// R2-20 round 2 features and their limits.
pub(crate) const TYPEWRITER_WAVES: &str = "typewriter-waves";
pub(crate) const ORBITING_WORDS: &str = "orbiting-words";
pub(crate) const BREATHING_SCROLL: &str = "breathing-scroll";
const WAVE_RING_SECS: f64 = 1.2;
const WAVE_RING_ALPHA: f32 = 0.2;
const WAVE_RING_RX: f32 = 70.0;
const REFLECT_SECS: f64 = 3.0;
const REFLECT_ALPHA: f32 = 0.12;
const COPY_SECS: f64 = 0.5;
/// The copy runs along its orbit for this share of `COPY_SECS`, then lands.
const COPY_RUN: f64 = 0.7;
const COPY_ALPHA: f32 = 0.25;
const ORBITS: usize = 3;
const ORBIT_ALPHA: f32 = 0.16;
const SCROLL_ALPHA: f32 = 0.10;
const SCROLL_FADE_SECS: f64 = 0.8;
const _: () = assert!(WAVE_RING_ALPHA <= 0.2 && COPY_ALPHA <= 0.25);

/// The projection in the redesign's units at `width_px = 600`.
const DESIGN_W: f32 = 600.0;
/// The microphone orb.
const ORB_Y: f32 = 82.0;
const ORB_R: f32 = 42.0;
/// The column of light, from under the orb down to the ring.
const BEAM_X0: f32 = 78.0;
const BEAM_X1: f32 = 522.0;
const BEAM_TOP: f32 = 140.0;
const CAP_RY: f32 = 16.0;
/// The words: centred in this width from this top, the wave and the ring
/// below the last line, so the projection grows down as lines come.
const TEXT_W: f32 = 420.0;
const TEXT_PX: f32 = 31.0;
const TEXT_LINE: f32 = 40.0;
const TEXT_TOP: f32 = 168.0;
const MAX_LINES: u32 = 5;
/// The waveform at the column's base, this far below the last line.
const WAVE_GAP: f32 = 46.0;
const WAVE_X0: f32 = 136.0;
const WAVE_X1: f32 = 464.0;
const WAVE_HALF: f32 = 30.0;
const BARS: usize = 49;
/// Seconds of audio between neighbouring bars, out from the middle.
const BAR_SECS: f64 = 0.03;
/// Levels under this are room tone and draw flat.
const WAVE_GATE: f32 = 0.04;
/// The ring pedestal, this far below the wave, and the status on it.
const RING_GAP: f32 = 82.0;
const RING_PAD: f32 = 40.0;
const RING_RX: f32 = 214.0;
const RING_RY: f32 = 26.0;
const STATUS_PX: f32 = 13.5;
/// Scan lines in the column: their pitch and how fast they drift down.
const SCAN_PITCH: f32 = 5.0;
const SCAN_SPEED: f32 = 6.0;
/// Idle: a brighter scan line passes down the column this often, and the
/// older lines float a pixel up and down, each a half period apart.
const SWEEP_SECS: f64 = 6.0;
const SWEEP_ALPHA: f32 = 0.12;
const FLOAT_SECS: f64 = 3.0;
const FLOAT_PX: f32 = 1.0;
/// Seconds a line that left the reading place waits before it floats.
const FLOAT_AFTER_SECS: f64 = 2.0;
const SETTLE_SECS: f64 = 0.35;

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn face(look: &Look) -> u32 {
    look.fonts
        .system("Google Sans Flex:medium")
        .or_else(|| look.fonts.system("Inter"))
        .unwrap_or(SANS)
}

/// The status font: Inter has the arrow a correction's detail shows.
fn ui(look: &Look) -> u32 {
    look.fonts.system("Inter").unwrap_or(SANS)
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = scale(look);
    TextStyle::new(face(look), TEXT_PX * k, TEXT_LINE * k)
        .tracking(-0.4 * k)
        .centered()
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

/// The wave's middle in design units with `lines` lines of words.
fn wave_y(lines: u32) -> f32 {
    TEXT_TOP + lines as f32 * TEXT_LINE + WAVE_GAP
}

fn ring_y(lines: u32) -> f32 {
    wave_y(lines) + RING_GAP
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    let h = ring_y(MAX_LINES) + RING_RY + RING_PAD;
    Size::new((DESIGN_W * k).ceil(), (h * k).ceil())
}

pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    let lines = lines.clamp(1, MAX_LINES);
    (TEXT_TOP + (lines as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// Points round an ellipse.
fn ellipse(c: Point, rx: f32, ry: f32) -> Contour {
    let n = 96;
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Point::new(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect();
    Contour::polyline(&pts, true)
}

/// The light: the column with its scan lines, the ring pedestal and the
/// orb, in the surface's space.
#[derive(Clone, Copy)]
struct Light {
    k: f32,
    /// The ring's middle in design units.
    ring: f32,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    still: bool,
}

impl Light {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing if !self.still => f64::INFINITY,
            _ => self.mode_at + SETTLE_SECS + 0.1,
        }
    }

    /// 1 for the full blue light, down to 0 for a cancel's grey.
    fn power(&self, t: f64) -> f32 {
        let change = if self.still {
            1.0
        } else {
            super::kit::ease_out((t - self.mode_at) / SETTLE_SECS)
        };
        match self.mode {
            Mode::Cancelled => 1.0 - change,
            Mode::Loading => 0.55,
            _ => 1.0,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let power = self.power(t);
        let tint = |c: u32| mix(hex(DIM), hex(c), power);
        let (x0, x1) = (r.x + BEAM_X0 * k, r.x + BEAM_X1 * k);
        let (top, ring) = (r.y + BEAM_TOP * k, r.y + self.ring * k);
        // The column: clear at the top, a faint blue towards the ring.
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(x0, top, x1 - x0, ring - top), 18.0 * k),
            Paint::linear(
                Point::new(0.0, top),
                tint(HALO).fade(0.03),
                Point::new(0.0, ring),
                tint(HALO).fade(0.16),
            ),
        ));
        // The column's top: a faint rim under the orb, like a glass cylinder.
        let cap = ellipse(
            Point::new((x0 + x1) / 2.0, top),
            (x1 - x0) / 2.0,
            CAP_RY * k,
        );
        p.fill([&cap], &StrokeStyle::solid(1.0, tint(HALO).fade(0.06)));
        p.stroke(
            &cap,
            &StrokeStyle::solid(1.0 * k, tint(ICE).fade(0.14 * power)),
        );
        // Scan lines drifting down the column, fainter higher up.
        let drift = if self.still {
            0.0
        } else {
            ((t - self.shown_at) as f32 * SCAN_SPEED * k).rem_euclid(SCAN_PITCH * k)
        };
        let mut y = top + drift;
        while y < ring - 4.0 * k {
            let u = (y - top) / (ring - top);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0 + 8.0 * k, y, x1 - x0 - 16.0 * k, 1.0), 0.0),
                Paint::solid(tint(ICE).fade(0.035 * u)),
            ));
            y += SCAN_PITCH * k;
        }
        // Idle: one brighter scan line passes down the column every few
        // seconds while the popup listens.
        if !self.still && self.mode == Mode::Listening {
            let u = ((t - self.shown_at) / SWEEP_SECS).fract() as f32;
            let y = top + u * (ring - top);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0 + 8.0 * k, y, x1 - x0 - 16.0 * k, 1.0), 0.0),
                Paint::solid(tint(ICE).fade(SWEEP_ALPHA)),
            ));
        }
        // The ring pedestal: a glowing ellipse, a soft fill and a wider halo.
        let c = Point::new(r.x + r.w / 2.0, ring);
        let shimmer = if self.still || self.mode != Mode::Listening {
            0.0
        } else {
            0.08 * ((t - self.shown_at) * std::f64::consts::TAU / 3.0).sin() as f32
        };
        p.fill(
            [&ellipse(c, RING_RX * k, RING_RY * k)],
            &StrokeStyle::solid(1.0, tint(BLUE).fade(0.26 * power)),
        );
        p.stroke(
            &ellipse(c, (RING_RX + 30.0) * k, (RING_RY + 9.0) * k),
            &StrokeStyle::solid(1.0 * k, tint(HALO).fade(0.25 * power)),
        );
        p.stroke(
            &ellipse(c, RING_RX * k, RING_RY * k),
            &StrokeStyle::solid(2.6 * k, tint(BLUE))
                .linear(
                    Point::new(c.x - RING_RX * k, 0.0),
                    tint(CYAN),
                    Point::new(c.x + RING_RX * k, 0.0),
                    tint(VIOLET),
                )
                .glow((8.0 + 30.0 * shimmer) * k),
        );
        // The microphone orb.
        let o = Point::new(c.x, r.y + ORB_Y * k);
        let rad = ORB_R * k;
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::glow(tint(HALO).fade(0.5 * power), 14.0 * k, 0.5),
        ));
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::solid(hex(ORB).fade(0.92)),
        ));
        p.stroke(
            &make::ring(o, rad),
            &StrokeStyle::solid(1.6 * k, tint(CYAN)).glow(5.0 * k),
        );
        p.stroke(
            &make::ring(o, rad * 0.78),
            &StrokeStyle::solid(1.0 * k, tint(BLUE).fade(0.6)),
        );
        let unit = rad / 24.0 * 0.9;
        let mic = Outline::new(&icons::mic(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(unit))
                .then(&Affine::translate(o.x, o.y)),
        );
        p.outline(&mic, &StrokeStyle::solid(1.8 * unit, tint(ICE)));
    }
}

/// The waveform at the column's base: bars mirrored out from the middle,
/// the newest audio in the middle, cyan to blue to violet.
#[derive(Clone)]
struct Wave {
    k: f32,
    levels: Levels,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
}

impl Wave {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => f64::INFINITY,
            _ => self.mode_at.max(now) + SETTLE_SECS + 0.1,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let cy = r.y + r.h / 2.0;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let change = super::kit::ease_out((t - self.mode_at) / SETTLE_SECS);
        let (height, grey) = match self.mode {
            Mode::Loading => (0.0, 0.4),
            Mode::Finishing => (1.0 - 0.6 * change, 0.0),
            Mode::Done => (0.4 * (1.0 - change), 0.0),
            Mode::Cancelled => (1.0 - change, change),
            Mode::Listening => (1.0, 0.0),
        };
        let mid = BARS / 2;
        let pitch = r.w / (BARS - 1) as f32;
        for i in 0..BARS {
            let d = i.abs_diff(mid);
            let reach = d as f32 / mid as f32;
            let at = t_eff - d as f64 * BAR_SECS;
            // Room tone stays flat, so a silence holds still.
            let raw = self.levels.at(at);
            let raw = if raw < WAVE_GATE { 0.0 } else { raw };
            let jitter = 0.5 + 0.5 * noise((at / BAR_SECS).round() as i64 as u32);
            let taper = (1.0 - reach.powf(1.4)).max(0.0);
            let level = (raw * 2.2).min(1.0) * jitter * taper * height;
            let half = (level * WAVE_HALF * k).max(1.0 * k);
            let x = r.x + i as f32 * pitch;
            let u = i as f32 / (BARS - 1) as f32;
            let c = if u < 0.5 {
                mix(hex(CYAN), hex(BLUE), 2.0 * u)
            } else {
                mix(hex(BLUE), hex(VIOLET), 2.0 * u - 1.0)
            };
            let c = mix(c, hex(DIM), grey).fade(1.0 - 0.4 * reach);
            if half <= 1.2 * k {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.1 * k,
                    Paint::solid(c),
                ));
                continue;
            }
            let bar = Rect::new(x - 1.2 * k, cy - half, 2.4 * k, 2.0 * half);
            p.shape(ShapeInstance::capsule(
                bar,
                Paint::glow(c.fade(0.45), 4.0 * k, 0.4),
            ));
            p.shape(ShapeInstance::capsule(bar, Paint::solid(c)));
        }
    }
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// MO-83 on a centred line, as on particles: the letters of a word drop
/// 2 px together and bounce, with no sideways nudge, drift or stagger. A
/// centred line moves as it grows, and those on top of it loosened the
/// reading line's spacing past its limit.
fn letters() -> wild_motion::LetterPhysics {
    wild_motion::LetterPhysics {
        nudge: 0.0,
        drift: 0.0,
        stagger: 0.0,
        drop: 2.0,
        ..wild_motion::LetterPhysics::default()
    }
}

/// The words' colour, for effects that draw letters of their own.
pub(crate) fn text_color() -> Color {
    hex(TEXT)
}

/// The look's transcript for the outline features.
fn hologram_paths(look: &Look) -> PathsText {
    PathsText {
        style: look.text_style(),
        width: look.text_width(),
        line: TEXT_LINE * scale(look),
        ink: hex(TEXT),
        nib: hex(CYAN),
    }
}

/// R2-20-a: a word's ring `age` seconds after it came: its size as a share
/// of the widest and its alpha; `None` before it and from 1.2 s on.
fn wave_ring(age: f64) -> Option<(f32, f32)> {
    let u = age / WAVE_RING_SECS;
    if !(0.0..1.0).contains(&u) {
        return None;
    }
    Some((ease_out(u), WAVE_RING_ALPHA * (1.0 - u as f32).powi(2)))
}

/// R2-20-b: a word's copy `age` seconds after the word came: how far along
/// its run on the orbit (0 to 1), how far it has landed on the word (0 to
/// 1), and its alpha; `None` before it and from `COPY_SECS` on.
fn copy_at(age: f64) -> Option<(f32, f32, f32)> {
    let u = age / COPY_SECS;
    if !(0.0..1.0).contains(&u) {
        return None;
    }
    let run = ease_out((u / COPY_RUN).min(1.0));
    let land = ((u - COPY_RUN) / (1.0 - COPY_RUN)).max(0.0) as f32;
    // Faint at once, gone as it settles on the word.
    let fade = (1.0 - ((land - 0.5) / 0.5).max(0.0)) * ((u / 0.1).min(1.0) as f32);
    Some((run, land, COPY_ALPHA * fade))
}

/// R2-20-c: the scroll's alpha `t` seconds in, once the text was done at
/// `done` (None while it is not).
fn scroll_alpha(done: Option<f64>, t: f64) -> f32 {
    match done {
        Some(d) => SCROLL_ALPHA * (1.0 - ((t - d) / SCROLL_FADE_SECS).clamp(0.0, 1.0)) as f32,
        None => SCROLL_ALPHA,
    }
}

/// The voice summed over time up to `t`, in level-seconds: the orbits'
/// pace. `sums` holds the running sum at each level frame, the newest at
/// `newest_at`.
fn level_sum(sums: &[f32], newest_at: f64, t: f64) -> f32 {
    let n = sums.len();
    if n == 0 {
        return 0.0;
    }
    let back = ((newest_at - t) * FRAME_HZ as f64).max(0.0);
    let i = (n as f64 - 1.0 - back).max(0.0);
    sums[i.floor() as usize]
}

/// A word on screen for the round 2 features: its id, its left and right
/// ends, its baseline and its row (0 the top row shown), in the surface's
/// space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    id: WordId,
    x0: f32,
    x1: f32,
    baseline: f32,
    row: usize,
}

/// A round 2 layer's drawing.
type Draw = Box<dyn Fn(f64, Rect, &mut Painter) + Send + Sync>;

/// R2-20-a: the pool under the newest line: the reflection of its words
/// and a ring per word.
struct Pool {
    /// The newest line's words: left, right.
    line: Vec<(f32, f32)>,
    /// Where the water lies.
    y: f32,
    /// Each recent word's middle and when it came.
    rings: Vec<(f32, f64)>,
    newest: f64,
    k: f32,
    still: bool,
}

impl Pool {
    fn until(&self) -> f64 {
        if self.still {
            return f64::NEG_INFINITY;
        }
        let ring = self.rings.iter().map(|r| r.1 + WAVE_RING_SECS);
        ring.fold(self.newest + REFLECT_SECS, f64::max)
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let (ox, oy) = (r.x, r.y + self.y);
        // The reflection: three short streaks under each word, wavering
        // for a while after a word, still after that.
        let s = if self.still {
            0.0
        } else {
            (t.min(self.newest + REFLECT_SECS)) as f32
        };
        for &(x0, x1) in &self.line {
            for i in 0..3 {
                let f = i as f32;
                let shrink = 0.12 * f * (x1 - x0);
                let wob = 2.0 * k * (2.1 * s + 1.7 * f + 0.01 * x0).sin();
                let y = oy + (3.0 + 5.0 * f) * k;
                p.shape(ShapeInstance::line(
                    Point::new(ox + x0 + shrink + wob, y),
                    Point::new(ox + x1 - shrink + wob, y),
                    1.4 * k,
                    Paint::solid(hex(CYAN).fade(REFLECT_ALPHA * (1.0 - 0.3 * f))),
                ));
            }
        }
        if self.still {
            return;
        }
        for &(x, born) in &self.rings {
            let Some((size, a)) = wave_ring(t - born) else {
                continue;
            };
            let rx = WAVE_RING_RX * k * (0.1 + 0.9 * size);
            let pts: Vec<Point> = (0..40)
                .map(|i| {
                    let th = i as f32 / 40.0 * std::f32::consts::TAU;
                    Point::new(ox + x + rx * th.cos(), oy + 6.0 * k + 0.2 * rx * th.sin())
                })
                .collect();
            p.stroke(
                &Contour::polyline(&pts, true),
                &StrokeStyle::solid(1.1 * k, hex(ICE).fade(a)),
            );
        }
    }
}

/// R2-20-b: the orbits, their satellites, and the copies of new words.
struct Orbits {
    /// Each orbit's middle and radii.
    rings: [(Point, f32, f32); ORBITS],
    sums: Arc<[f32]>,
    newest_at: f64,
    /// The last voice above room tone: the orbits stand still after it.
    loud_until: f64,
    /// Each new word's copy: its glyphs (pen place from the word's middle
    /// on the baseline), where it lands (the middle on the baseline) and
    /// when the word came.
    copies: Vec<(Vec<GlyphInstance>, Point, f64)>,
    until: f64,
    k: f32,
    still: bool,
}

impl Orbits {
    fn until(&self) -> f64 {
        if self.still {
            return f64::NEG_INFINITY;
        }
        self.until.max(self.loud_until)
    }

    fn angle(&self, i: usize, t: f64) -> f32 {
        let start = i as f32 * 2.1;
        if self.still {
            return start;
        }
        // Only the voice moves them, so room tone draws nothing.
        let s = level_sum(&self.sums, self.newest_at, t.min(self.loud_until));
        start + 1.2 * s * if i.is_multiple_of(2) { 1.0 } else { -1.0 }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let o = Point::new(r.x, r.y);
        for (i, &(c, rx, ry)) in self.rings.iter().enumerate() {
            let c = Point::new(o.x + c.x, o.y + c.y);
            let pts: Vec<Point> = (0..64)
                .map(|j| {
                    let th = j as f32 / 64.0 * std::f32::consts::TAU;
                    Point::new(c.x + rx * th.cos(), c.y + ry * th.sin())
                })
                .collect();
            p.stroke(
                &Contour::polyline(&pts, true),
                &StrokeStyle::solid(0.9 * k, hex(BLUE).fade(ORBIT_ALPHA)),
            );
            let th = self.angle(i, t);
            p.shape(ShapeInstance::circle(
                Point::new(c.x + rx * th.cos(), c.y + ry * th.sin()),
                2.4 * k,
                Paint::glow(hex(ICE).fade(0.5), 4.0 * k, 0.6),
            ));
        }
        if self.still {
            return;
        }
        for (glyphs, to, born) in &self.copies {
            let Some((run, land, alpha)) = copy_at(t - born) else {
                continue;
            };
            // The orbit nearest the word's line; the copy runs on its back
            // half, above the word, and comes down onto it.
            let to = Point::new(o.x + to.x, o.y + to.y);
            let (c, rx, ry) = self
                .rings
                .iter()
                .map(|&(c, rx, ry)| (Point::new(o.x + c.x, o.y + c.y), rx, ry))
                .min_by(|a, b| (a.0.y - to.y).abs().total_cmp(&(b.0.y - to.y).abs()))
                .unwrap_or((to, 1.0, 1.0));
            let end = ((to.x - c.x) / rx).clamp(-1.0, 1.0).acos();
            let start = end + 1.6;
            let th = start + (end - start) * run;
            let on = Point::new(c.x + rx * th.cos(), c.y - ry * th.sin());
            let at = Point::new(on.x + (to.x - on.x) * land, on.y + (to.y - on.y) * land);
            // Turned with the orbit while it runs, straight as it lands.
            let tangent = (-ry * th.cos()).atan2(-rx * th.sin());
            let turn = tangent.to_degrees().clamp(-35.0, 35.0) * (1.0 - land);
            let (sin, cos) = turn.to_radians().sin_cos();
            for g in glyphs {
                let (dx, dy) = (g.pos.x, g.pos.y);
                p.glyph(GlyphInstance {
                    pos: Point::new(at.x + dx * cos - dy * sin, at.y + dx * sin + dy * cos),
                    color: hex(TEXT).fade(alpha),
                    xform: GlyphXform::rotate(turn),
                    ..*g
                });
            }
        }
    }
}

/// R2-20-c: the scroll behind the text, from `top` to `bottom`, `x0` to
/// `x1`, curled at its top left and foot right.
struct Scroll {
    x0: f32,
    x1: f32,
    top: f32,
    bottom: f32,
    levels: Levels,
    /// The last voice above room tone: the sides rest after it.
    loud_until: f64,
    done: Option<f64>,
    k: f32,
    still: bool,
}

impl Scroll {
    fn until(&self) -> f64 {
        let fade = self
            .done
            .map_or(f64::NEG_INFINITY, |d| d + SCROLL_FADE_SECS);
        if self.still {
            return fade;
        }
        fade.max(self.loud_until)
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let a = scroll_alpha(self.done, t);
        if a <= 0.0 {
            return;
        }
        let k = self.k;
        let (ox, oy) = (r.x, r.y);
        let (x0, x1, top, bottom) = (ox + self.x0, ox + self.x1, oy + self.top, oy + self.bottom);
        // The sides bow out a little with the voice.
        let bow = if self.still {
            0.0
        } else {
            6.0 * k * self.levels.at(t.min(self.loud_until)).clamp(0.0, 1.0)
        };
        let n = 16;
        let side = |x: f32, sign: f32| -> Vec<Point> {
            (0..=n)
                .map(|i| {
                    let u = i as f32 / n as f32;
                    let y = top + (bottom - top) * u;
                    Point::new(x + sign * bow * (std::f32::consts::PI * u).sin(), y)
                })
                .collect()
        };
        let mut pts = side(x0, -1.0);
        pts.extend(side(x1, 1.0).into_iter().rev());
        let sheet = Contour::polyline(&pts, true);
        p.fill(
            [&sheet],
            &StrokeStyle::solid(0.0, hex(BLUE).fade(a)).linear(
                Point::new(0.0, top),
                hex(BLUE).fade(a),
                Point::new(0.0, bottom),
                hex(VIOLET).fade(a * 0.6),
            ),
        );
        p.stroke(
            &sheet,
            &StrokeStyle::solid(1.0 * k, hex(CYAN).fade(a * 1.6)),
        );
        if self.still {
            return;
        }
        // The curls: a rolled edge at the top left and at the foot right.
        let curl = 6.0 * k;
        for (c, w) in [
            (Point::new(x0 + 0.18 * (x1 - x0), top), 0.3 * (x1 - x0)),
            (Point::new(x0 + 0.82 * (x1 - x0), bottom), 0.3 * (x1 - x0)),
        ] {
            p.shape(ShapeInstance::capsule(
                Rect::new(c.x - w / 2.0, c.y - curl, w, 2.0 * curl),
                Paint::linear(
                    Point::new(c.x, c.y - curl),
                    hex(ICE).fade(a * 1.2),
                    Point::new(c.x, c.y + curl),
                    hex(BLUE).fade(a * 0.5),
                ),
            ));
        }
    }
}

impl Popup {
    pub(crate) fn hologram_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.hologram_projection())
    }

    fn hologram_projection(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let size = surface(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let cancelled = mode == Mode::Cancelled;
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let lines = self.lines.clamp(1, MAX_LINES);
        let light = Light {
            k,
            ring: ring_y(lines),
            shown_at: self.shown_at,
            mode,
            mode_at,
            still: self.still(),
        };
        let wave = Wave {
            k,
            levels: Levels {
                levels: self.exhale_levels(&self.kit_levels().levels),
                newest_at: self.newest_at,
            },
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
        };
        let wave_until = wave.until(self.extra.now);
        let text_top = TEXT_TOP * k;
        let text_x = (DESIGN_W - TEXT_W) / 2.0 * k;
        Element::layer("hologram")
            .size(size.w, size.h)
            .presence(Transition::fade().offset(0.0, 8.0), exit)
            .child(at(
                Element::canvas("light", light.until(), move |t, r, p| light.draw(t, r, p))
                    .size(size.w, size.h),
                0.0,
                0.0,
            ))
            .child(
                at(
                    Element::canvas("wave", wave_until, move |t, r, p| wave.draw(t, r, p))
                        .size((WAVE_X1 - WAVE_X0) * k, 2.0 * WAVE_HALF * k),
                    WAVE_X0 * k,
                    (wave_y(lines) - WAVE_HALF) * k,
                )
                // The wave and the ring move down with a new line at once, so
                // the line never draws under them.
                .motion(Prop::Position, Motion::Snap),
            )
            .children(self.limit_line(
                Point::new(text_x, text_top),
                size,
                hex(TEXT).fade(0.55),
                hex(CYAN),
            ))
            .children(self.hologram_round2(mode, mode_at, lines, k, text_x))
            .child(self.hologram_text_clip(mode, lines, k, text_x))
            .child(at(
                self.hologram_status(mode, mode_at, k),
                (DESIGN_W / 2.0 - RING_RX) * k,
                (ring_y(lines) - 10.0) * k,
            ))
    }

    /// The round 2 layers behind and under the words: the scroll, the
    /// orbits and the pool, each when its feature is on. None while
    /// editing, on a cancel or in the depth stack.
    fn hologram_round2(
        &self,
        mode: Mode,
        mode_at: f64,
        lines: u32,
        k: f32,
        text_x: f32,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        let any = [TYPEWRITER_WAVES, ORBITING_WORDS, BREATHING_SCROLL]
            .iter()
            .any(|f| l.on(f));
        if !any
            || self.edit_view.is_some()
            || mode == Mode::Cancelled
            || l.on(wild_space::DEPTH_STACK)
        {
            return Vec::new();
        }
        let size = surface(l);
        let still = self.still();
        let now = self.extra.now;
        let placed = self.hologram_placed(lines, k, text_x);
        let born = |id: WordId| self.heard.word(id).map_or(self.shown_at, |h| h.born);
        let text_top = TEXT_TOP * k;
        let text_h = lines as f32 * TEXT_LINE * k;
        let levels = Levels {
            levels: self.exhale_levels(&self.kit_levels().levels),
            newest_at: self.newest_at,
        };
        let quiet_frames = levels
            .levels
            .iter()
            .rev()
            .take_while(|&&v| v <= WAVE_GATE)
            .count();
        let loud_until = self.newest_at - quiet_frames as f64 / FRAME_HZ as f64;
        let mut out = Vec::new();
        let canvas = |key: &'static str, until: f64, draw: Draw| {
            at(
                Element::canvas(key, until, move |t, r, p| draw(t, r, p))
                    .size(size.w, size.h)
                    .presence(Transition::fade(), Transition::fade()),
                0.0,
                0.0,
            )
        };
        if l.on(BREATHING_SCROLL) {
            let pad = 18.0 * k;
            let scroll = Scroll {
                x0: text_x - pad,
                x1: text_x + TEXT_W * k + pad,
                top: text_top - 12.0 * k,
                bottom: text_top + text_h + 8.0 * k,
                levels: levels.clone(),
                loud_until,
                done: (mode == Mode::Done).then_some(mode_at),
                k,
                still,
            };
            out.push(canvas(
                "scroll",
                scroll.until(),
                Box::new(move |t, r, p| scroll.draw(t, r, p)),
            ));
        }
        if l.on(ORBITING_WORDS) {
            let mid = size.w / 2.0;
            let rings = std::array::from_fn(|i| {
                let f = [0.2, 0.55, 0.9][i];
                (
                    Point::new(mid, text_top + text_h * f),
                    (0.5 - 0.04 * i as f32) * TEXT_W * k,
                    14.0 * k,
                )
            });
            let mut sum = 0.0;
            let sums: Arc<[f32]> = levels
                .levels
                .iter()
                .map(|&v| {
                    sum += (v - WAVE_GATE).clamp(0.0, 1.0) / FRAME_HZ as f32;
                    sum
                })
                .collect();
            let copies: Vec<_> = if still {
                Vec::new()
            } else {
                placed
                    .iter()
                    .filter(|w| born(w.id) + COPY_SECS > now)
                    .filter_map(|w| {
                        let glyphs = self.hologram_copy(w.id)?;
                        let to = Point::new((w.x0 + w.x1) / 2.0, w.baseline);
                        Some((glyphs, to, born(w.id)))
                    })
                    .collect()
            };
            let until = copies
                .iter()
                .map(|c| c.2 + COPY_SECS)
                .fold(f64::NEG_INFINITY, f64::max);
            let orbits = Orbits {
                rings,
                sums,
                newest_at: self.newest_at,
                loud_until,
                copies,
                until,
                k,
                still,
            };
            out.push(canvas(
                "orbits",
                orbits.until(),
                Box::new(move |t, r, p| orbits.draw(t, r, p)),
            ));
        }
        if l.on(TYPEWRITER_WAVES) {
            if let Some(last) = placed.last() {
                let line = placed
                    .iter()
                    .filter(|w| w.row == last.row)
                    .map(|w| (w.x0, w.x1))
                    .collect();
                let rings = placed[placed.len().saturating_sub(6)..]
                    .iter()
                    .map(|w| ((w.x0 + w.x1) / 2.0, born(w.id)))
                    .collect();
                let newest = placed
                    .iter()
                    .map(|w| born(w.id))
                    .fold(self.shown_at, f64::max);
                let pool = Pool {
                    line,
                    y: last.baseline + 8.0 * k,
                    rings,
                    newest,
                    k,
                    still,
                };
                out.push(canvas(
                    "pool",
                    pool.until(),
                    Box::new(move |t, r, p| pool.draw(t, r, p)),
                ));
            }
        }
        out
    }

    /// The shown words on the shown rows, centred as the live text sets
    /// them, in the surface's space.
    fn hologram_placed(&self, lines: u32, k: f32, text_x: f32) -> Vec<Placed> {
        let l = &self.look;
        let tw = text_width(l);
        let (boxes, total) =
            sounding::word_boxes(&self.measure, &self.shown, &l.text_style(), Some(tw));
        let first = total.saturating_sub(lines as usize);
        // The layout sets lines from the left; the live text centres each
        // one in the column.
        let mut shift = HashMap::new();
        for b in &boxes {
            let e = shift.entry(b.line).or_insert((f32::MAX, f32::MIN));
            e.0 = e.0.min(b.rect.x);
            e.1 = e.1.max(b.rect.x + b.rect.w);
        }
        let y0 = TEXT_TOP * k - first as f32 * TEXT_LINE * k;
        boxes
            .iter()
            .filter(|b| b.line >= first)
            .map(|b| {
                let (x0, x1) = shift[&b.line];
                let s = text_x + (tw - (x1 - x0)) / 2.0 - x0;
                Placed {
                    id: b.id,
                    x0: b.rect.x + s,
                    x1: b.rect.x + b.rect.w + s,
                    baseline: y0 + b.baseline,
                    row: b.line - first,
                }
            })
            .collect()
    }

    /// R2-20-b: the glyphs of word `id` for its copy, placed from the
    /// middle of its ink on its baseline.
    fn hologram_copy(&self, id: WordId) -> Option<Vec<GlyphInstance>> {
        let w = self.shown.iter().find(|w| w.id == id)?;
        let l = &self.look;
        let k = scale(l);
        let style = TextStyle::new(face(l), TEXT_PX * k, TEXT_LINE * k).tracking(-0.4 * k);
        let shaped = self.measure.layout(&w.text, &style, None);
        let base = shaped
            .lines
            .first()
            .map_or(0.8 * style.size, |l| l.baseline);
        let outline = WordOutline::new(self.measure.fonts(), &shaped, style.size, Point::ZERO);
        let mid = outline.ink.x + outline.ink.w / 2.0;
        Some(
            outline
                .instances(hex(TEXT))
                .map(|g| GlyphInstance {
                    pos: Point::new(g.pos.x - mid, g.pos.y - base),
                    word: NO_WORD,
                    ..g
                })
                .collect(),
        )
    }

    /// The words, clipped to the column under the orb, so a line that
    /// leaves on an overflow fades out inside the light.
    fn hologram_text_clip(&self, mode: Mode, lines: u32, k: f32, text_x: f32) -> Element<Msg> {
        // Across the whole projection: a centred line that grows slides to
        // its new middle, and a new word rides along past the text box
        // until it gets there.
        let m = 10.0 * k;
        let top = BEAM_TOP + 6.0;
        let y = (TEXT_TOP - top) * k;
        let h = y + lines as f32 * TEXT_LINE * k + m;
        let clip = Element::layer("text-clip")
            .size(DESIGN_W * k, h)
            .clip()
            .motion(Prop::Size, Motion::Snap)
            .child(at(self.hologram_text(mode, lines), text_x, y));
        at(clip, 0.0, top * k)
    }

    fn hologram_text(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let stacking = l.on(wild_space::DEPTH_STACK);
        let paths = hologram_paths(l);
        let words = if stacking {
            self.hologram_stack(mode, lines)
        } else {
            // R2-20-b: with the orbits a word arrives at 70 % ink on its
            // baseline; its copy brings it in.
            let enter = if l.on(ORBITING_WORDS) {
                Transition::fade().opacity(0.7)
            } else {
                Transition::fade().opacity(0.0).offset(0.0, -8.0 * k)
            };
            let words = self
                .hologram_words(&self.shown, mode)
                .word_enter(self.ghost_enter(enter));
            self.typo_words_in(self.hologram_letters(words), &paths)
        };
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            hex(TEXT),
            hex(CYAN),
        );
        let editing = self.edit_view.is_some();
        let text = Element::column("text")
            .width(l.text_width())
            .height(lines as f32 * TEXT_LINE * k)
            .child(words)
            // The pen draws at the flat layout's places, which the depth
            // stack does not use: with the stack it drew loose outlines.
            .children(
                self.paths_overlays(&paths)
                    .into_iter()
                    .filter(|_| !editing && !stacking),
            )
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink {
                    text: hex(TEXT),
                    mark: hex(CYAN),
                    faint: hex(DIM),
                },
                endings::Cancel::Unthread,
            ));
        // MO-87: a revised word flows into its correction over the words.
        let text = self.letter_fx_layers(text, l.text_width());
        // The depth stack lays its lines out on its own and carries the
        // ghost tail on its reading line.
        let text = if stacking {
            text
        } else {
            self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), hex(TEXT))
        };
        self.with_hearing(text, &l.text_style(), l.text_width(), hex(TEXT), hex(CYAN))
    }

    /// The live text of `words` in the look's projected type.
    fn hologram_words(&self, words: &[Word], mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.4 } else { 1.0 };
        Element::live_text("words", words)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(TEXT))
            .text_halo(hex(HALO).fade(0.55), 9.0 * scale(l))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
    }

    /// The letter effects on the reading line's live text: the liquid
    /// morph's swaps, and letter physics tuned for a centred line.
    fn hologram_letters(&self, words: Element<Msg>) -> Element<Msg> {
        let words = if self.look.on(wild_motion::LETTER_PHYSICS) {
            // The letters' drop moves the glyphs: a revision swaps them at
            // once, as on particles.
            words.word_revise(vo::scene::flow::Revise::Swap)
        } else {
            words
        };
        self.letter_fx_words(words)
            .wild_motion_with(self, letters())
    }

    /// Whether the older lines float: while the popup listens, not under
    /// reduced motion, and only with older lines on the depth stack's
    /// sheets to move. The flat text has one live text for all its lines,
    /// so its older lines hold still.
    fn hologram_floats(&self) -> bool {
        self.look.theme == OsdTheme::Hologram
            && self.look.on(wild_space::DEPTH_STACK)
            && self.lines > 1
            && !self.closing
            && !self.still()
            && self.kit_mode() == Mode::Listening
    }

    /// Idle: how far the line `d` places behind the reading line floats now,
    /// a pixel up or down, each line a half period after the one in front.
    /// The reading line never moves.
    fn hologram_float(&self, d: usize) -> f32 {
        if d == 0 || !self.hologram_floats() {
            return 0.0;
        }
        // The line that just left the reading place settles first.
        let settling = self
            .jarvis
            .hold
            .as_ref()
            .is_some_and(|h| self.extra.now - h.since < FLOAT_AFTER_SECS);
        if d == 1 && settling {
            return 0.0;
        }
        let half = FLOAT_SECS / 2.0;
        let step = ((self.extra.now - self.shown_at) / half).floor() as i64;
        let up = (step + d as i64) % 2 == 0;
        let k = scale(&self.look);
        if up {
            -FLOAT_PX * k
        } else {
            FLOAT_PX * k
        }
    }

    /// Wakes the view at each half period while the older lines float.
    pub(crate) fn arm_hologram_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.extra.tick.is_some() || !self.hologram_floats() {
            return;
        }
        let half = FLOAT_SECS / 2.0;
        let next = half - (cx.t() - self.shown_at).rem_euclid(half);
        self.extra.tick = Some(cx.after(next + 0.01, Msg::Tick));
    }

    /// MO-86: the transcript as a depth stack. The reading line lies flat
    /// at the bottom row; older lines recede up and back on sheets of blue
    /// glass, dimmer and softer with each place, and float while idle.
    fn hologram_stack(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let style = l.text_style();
        let w = l.text_width();
        let line_h = style.line_height;
        let reduced = self.wild_reduced();
        // A little wider than the text, so a full line sits inside its sheet.
        let sheet = Size::new(w + 12.0 * k, line_h + 8.0 * k);
        let stack = DepthStack {
            step_z: 90.0 * k,
            // A little over a line pitch per place, the same at every place:
            // the stack's perspective draws each rise smaller, and with less
            // the lines further back sit on the ones in front.
            step_rise: line_h * 1.15,
            parallax: vec![1.0],
            // Every sheet moves on one spring: a deeper sheet that lagged was
            // still in its old place when the one in front rose into it.
            slowdown: 0.0,
            step_x: 0.0,
            sheet: Some(Sheet {
                fill: hex(ORB).fade(0.45),
                edge: hex(CYAN).fade(0.35),
                edge_width: 1.0,
                radius: 6.0 * k,
                pad: (0.0, 4.0 * k),
            }),
            ..DepthStack::default()
        };
        let all = &self.jarvis.lines;
        let shown = &all[all.len().saturating_sub(lines as usize)..];
        let keyed: Vec<(vo::scene::Key, &(Vec<Word>, f32))> = shown
            .iter()
            .map(|line| {
                let first = line.0.first().map_or(0, |w| w.id);
                (wild_space::depth_stack_key(first), line)
            })
            .collect();
        let tail = |room: f32| model_signals::TailText {
            style,
            width: room,
            rows: 1,
        };
        let content = |line: &&(Vec<Word>, f32), d: usize, p: &Place| {
            let (words, width) = *line;
            let lw = width.ceil() + 2.0;
            // A word lands in place at full ink: the sheet of a new line
            // already rises from 70 % ink (as on jarvis).
            let live = self
                .hologram_words(words, mode)
                .text_blur(p.blur)
                .motion(Prop::TextFx, stack.motion(d, reduced))
                .size(lw, line_h);
            let live = if d == 0 {
                self.typo_words(self.hologram_letters(live))
            } else {
                live
            };
            let live = live
                .word_enter(Transition::NONE)
                .word_change(Transition::fade().opacity(0.7));
            let line_e = Element::layer("line").size(lw, line_h).child(live);
            let line_e = if d == 0 {
                self.signal_marks_in(
                    line_e,
                    words,
                    &tail((sheet.w + lw) / 2.0),
                    (0.0, 0.0),
                    hex(TEXT),
                )
            } else {
                line_e
            };
            at(
                line_e,
                ((sheet.w - lw) / 2.0).max(0.0),
                4.0 * k + self.hologram_float(d),
            )
            .motion(Prop::Position, Spring::SOFT)
        };
        let h = lines.max(1) as f32 * line_h;
        let x = (w - sheet.w) / 2.0;
        let view = at(
            stack.view_at(
                "depth-stack",
                Size::new(sheet.w, h),
                sheet,
                &keyed,
                content,
                reduced,
                self.jarvis.hold.as_ref(),
                self.extra.now,
            ),
            x,
            0.0,
        );
        let stack_box = Element::column("stack-box").size(w, h).child(view);
        if !keyed.is_empty() {
            return stack_box;
        }
        // No line yet: the forming cue where the first line lands.
        self.signal_marks_in(stack_box, &[], &tail(w), (0.0, h - line_h), hex(TEXT))
    }

    /// The status, centred on the ring: a dot, the state word and the
    /// detail or hint.
    fn hologram_status(&self, mode: Mode, mode_at: f64, k: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(ui(l), STATUS_PX * k, 20.0 * k);
        let status = self.status();
        let (state, detail, dot) = match mode {
            Mode::Done => {
                let detail = self
                    .kit_target()
                    .map(|a| format!("into {a}"))
                    .unwrap_or_default();
                (self.delivery_word().to_string(), detail, hex(GREEN))
            }
            Mode::Cancelled => (
                "Cancelled".to_string(),
                "  ·  nothing was typed".to_string(),
                hex(DIM),
            ),
            _ => (status.state.clone(), status.detail.clone(), hex(CYAN)),
        };
        let hollow = mode == Mode::Finishing;
        let mark = Element::canvas("dot", mode_at, move |_, r, p| {
            let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            if hollow {
                p.stroke(
                    &make::ring(c, 4.6 * k),
                    &StrokeStyle::solid(1.3 * k, hex(ICE)),
                );
            } else {
                p.shape(ShapeInstance::circle(
                    c,
                    4.6 * k,
                    Paint::glow(dot.fade(0.5), 4.0 * k, 0.5),
                ));
                p.shape(ShapeInstance::circle(c, 4.6 * k, Paint::solid(dot)));
            }
        })
        .size(12.0 * k, 12.0 * k)
        .motion(Prop::Position, Motion::Snap);
        let words = super::kit::detail_words(&state, &detail);
        let mut row = Element::row("status")
            .width(2.0 * RING_RX * k)
            .height(20.0 * k)
            .align_items(AlignItems::CENTER)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
            .child(
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(hex(ICE))
                    .margin_edges(0.0, 0.0, 0.0, 9.0 * k)
                    .motion(Prop::Position, Motion::Snap),
            )
            .child(
                Element::live_text(format!("detail-{state}"), &words)
                    .text_style(style)
                    .color(hex(HINT))
                    .height(20.0 * k)
                    .margin_edges(0.0, 0.0, 0.0, 3.0 * k)
                    .shrink(1.0)
                    .min_width(0.0)
                    .motion(Prop::Position, Motion::Snap)
                    .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                    .word_enter(Transition::fade())
                    .word_change(Transition::fade().opacity(0.2)),
            );
        if let Some(fraction) = self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))
        {
            row = row.child(
                Element::progress(
                    "progress",
                    Progress {
                        fraction: Some(fraction),
                        track: hex(BLUE).fade(0.2),
                        bar: hex(CYAN),
                        since: self.shown_at,
                        period: 1.6,
                    },
                )
                .height(2.0 * k)
                .width(80.0 * k)
                .margin_edges(0.0, 0.0, 0.0, 12.0 * k),
            );
        }
        row
    }
}

#[cfg(test)]
mod round2_tests {
    use super::*;

    #[test]
    fn a_ring_per_word_gone_in_1_2_s() {
        assert!(wave_ring(-0.01).is_none());
        assert!(wave_ring(WAVE_RING_SECS).is_none());
        let (s0, a0) = wave_ring(0.0).unwrap();
        assert!(s0 == 0.0 && a0 <= WAVE_RING_ALPHA);
        let (s1, a1) = wave_ring(0.6).unwrap();
        assert!(s1 > s0 && a1 < a0, "the ring spreads and fades");
    }

    #[test]
    fn the_copy_runs_lands_and_is_gone_500_ms_after_the_word() {
        assert!(copy_at(-0.01).is_none());
        assert!(copy_at(COPY_SECS).is_none());
        for i in 0..50 {
            let (_, _, a) = copy_at(i as f64 * 0.01).unwrap();
            assert!(a <= COPY_ALPHA);
        }
        let (run, land, _) = copy_at(COPY_RUN * COPY_SECS * 0.5).unwrap();
        assert!(run > 0.0 && run < 1.0 && land == 0.0, "on its orbit first");
        let (run, land, a) = copy_at(COPY_SECS - 0.001).unwrap();
        assert!(run == 1.0 && land > 0.99 && a < 0.01, "on the word, faded");
    }

    #[test]
    fn the_scroll_fades_once_the_text_is_done() {
        assert_eq!(scroll_alpha(None, 5.0), SCROLL_ALPHA);
        assert_eq!(scroll_alpha(Some(2.0), 2.0), SCROLL_ALPHA);
        assert!(scroll_alpha(Some(2.0), 2.4) < SCROLL_ALPHA);
        assert_eq!(scroll_alpha(Some(2.0), 2.0 + SCROLL_FADE_SECS + 1e-9), 0.0);
    }

    #[test]
    fn the_orbits_pace_is_the_voice_summed() {
        let sums: Vec<f32> = (1..=10).map(|i| i as f32).collect();
        assert_eq!(level_sum(&sums, 1.0, 1.0), 10.0);
        assert!(level_sum(&sums, 1.0, 0.9) < 10.0);
        assert_eq!(level_sum(&sums, 1.0, -100.0), 1.0);
        assert_eq!(level_sum(&[], 1.0, 1.0), 0.0);
    }
}
