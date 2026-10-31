//! The `mecha` look of the vo popup: an anime mecha cockpit. An orange
//! angular frame with notches and hexagon plates, a bilingual title, hazard
//! stripes, a big mirrored bar waveform, a level meter and small readouts,
//! with the transcript in heavy condensed type.
//!
//! One canvas draws everything but the transcript, from the frame time: the
//! frame draws itself on when the popup opens, the plates snap in, the title
//! tracks in and the labels type in; while listening the stripes slide and
//! the bars scroll; finishing freezes the bars under a scan line; done lays
//! them flat; a cancel turns the frame red and strikes the text through.
//!
//! The drawing is in the units of the concept it was made from, 980 by 630,
//! scaled to the popup's width.
//!
//! Round 2 (R2-23), each an opt-in motion feature:
//!
//! - `magnetic-ink`: when a word comes, faint particles flow in from the
//!   panel's left edge along a curling path, gather behind the word in
//!   300 ms and disperse in 500 ms. The word itself is at 70 % ink or more
//!   from its first frame. Reduced motion: no particles.
//! - `glass-prism`: the text panel is a glass slab (EG-06, shaped to the
//!   panel, no refraction), and each new word sends one spectral band
//!   across it, left to right, in 600 ms, at 15 % or less over the text.
//!   Reduced motion: the slab and no sweep.
//! - `breathing-grid`: faint contour lines behind the text rise round the
//!   newest word by up to 4 px and settle in 800 ms, and ripple with the
//!   voice. Reduced motion: still lines.
//!
//! The grid is drawn on a canvas from the word boxes, not with the EG-31
//! text field, which needs an app material and host support the popup
//! does not have.

use super::*;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use vo::scene::canvas::Painter;
use vo::scene::display::{BlendMode, GlyphInstance, Paint, ShapeInstance, NO_WORD};
use vo::scene::frame::Corner;
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::{Glass, GlassShape};
use vo::scene::path::{icons, Contour, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo::scene::taffy::JustifyContent;
use vo::scene::text::ShapedText;

/// The drawing's own size, in concept units.
const CW: f32 = 980.0;
const CH: f32 = 630.0;
/// The popup is never narrower than this: below it the labels get too small.
const MIN_WIDTH: f32 = 600.0;
/// Transcript lines. The frame is fixed; older words leave at the top.
pub(crate) const MAX_LINES: u32 = 3;

// Transcript box, in concept units.
const TEXT_X: f32 = 140.0;
const TEXT_Y: f32 = 312.0;
const TEXT_W: f32 = 720.0;
const TEXT_PX: f32 = 56.0;
const TEXT_LINE: f32 = 65.0;

// Waveform box.
const WAVE_X0: f32 = 136.0;
const WAVE_X1: f32 = 896.0;
const WAVE_CY: f32 = 206.0;
const WAVE_HALF: f32 = 86.0;
const BAR_PITCH: f32 = 6.4;
const BAR_W: f32 = 4.4;

const ORANGE: u32 = 0xff7417;
const DEEP: u32 = 0xff4a00;
const YELLOW: u32 = 0xffb81c;
const TEXT: u32 = 0xff8a24;
const HINT: u32 = 0xffc9a0;
const FILL: u32 = 0x090504;
const PANEL: u32 = 0x070403;
const RED: u32 = 0xff2a1e;

/// MO-70: a new word pushes the caret and the status plate; a revision
/// slides the word's plate out and back.
pub(crate) const HYDRAULIC_CHAIN: &str = "hydraulic-chain";
/// MO-73: an onset opens a joint in the caret's foot that becomes the next
/// word's baseline tick.
pub(crate) const ONSET_JOINT: &str = sounding::ONSET_JOINT;

/// The lag between two links of the hydraulic chain: word, caret, plate.
const LINK: f64 = 0.015;
/// How far a new word pushes the caret on past its place, in ems.
const CARET_KICK: f32 = 0.12;
/// How far one word pushes the status plate, in concept units.
const PUSH: f32 = 5.0;

/// Idle motion (RD-02), while listening. The level meter's cell just above
/// the reading flickers this much above its unlit ink, a new value this
/// many times a second, as a gauge needle jitters about its reading.
const JITTER_INK: f32 = 0.12;
const JITTER_HZ: f64 = 6.0;
/// The two side bars left of the text panel reach toward each other by this
/// many pixels and back on this cycle, as a hydraulic pair breathing.
const BREATHE_PX: f32 = 2.0;
const BREATHE_SECS: f64 = 3.0;

/// R2-23-a: particles per word at a quiet and a loud word, their alpha,
/// the seconds they take to flow in and to disperse, how far they curl off
/// the straight path and how far they drift apart (concept units).
pub(crate) const MAGNETIC_INK: &str = "magnetic-ink";
const INK_DOTS: (f32, f32) = (40.0, 110.0);
const INK_ALPHA: f32 = 0.3;
const INK_IN: f64 = 0.3;
const INK_OUT: f64 = 0.5;
const INK_CURL: f32 = 40.0;
const INK_DRIFT: f32 = 46.0;
/// R2-23-b: the band's crossing time, its alpha, width (concept units) and
/// colours, left to right.
pub(crate) const GLASS_PRISM: &str = "glass-prism";
const SWEEP_SECS: f64 = 0.6;
const SWEEP_ALPHA: f32 = 0.15;
const SWEEP_W: f32 = 120.0;
const SWEEP_COLOURS: [u32; 5] = [0xff4d6d, 0xffb340, 0xfff06a, 0x5cf0c8, 0x6f9bff];
/// R2-23-c: the contour lines' pitch (concept units) and alpha, the
/// newest word's lift (pixels) and its time to settle, how far round the
/// word the lift reaches (concept units), and the voice's ripple (pixels).
pub(crate) const BREATHING_GRID: &str = "breathing-grid";
const GRID_PITCH: f32 = 16.0;
const GRID_ALPHA: f32 = 0.12;
const GRID_LIFT: f32 = 4.0;
const GRID_SECS: f64 = 0.8;
const GRID_REACH: f32 = 70.0;
const GRID_WAVE: f32 = 1.5;
/// Room tone counts as no voice.
const VOICE_GATE: f32 = 0.04;

/// The fonts of the look.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fonts {
    /// The transcript: heavy, extra condensed.
    text: u32,
    /// Labels and the status line.
    label: u32,
    /// Small capitals.
    small: u32,
    /// Japanese.
    cjk: u32,
}

/// The look's fonts, added to `reg` once (`FontRegistry::asset` and
/// `system`), so every registry, such as a test's, has its own ids. Labels
/// use the framework's built-in Barlow Condensed and Japanese label subset;
/// the transcript uses Saira ExtraCondensed ExtraBold, built in when the
/// framework has it, else from fontconfig, else the built-in condensed bold.
fn fonts(reg: &FontRegistry) -> Fonts {
    let asset = |name: &str, fallback: u32| reg.asset(name).unwrap_or(fallback);
    let label = asset("condensed-bold", SANS);
    let small = asset("condensed", MONO);
    let cjk = asset("jp-labels-bold", SANS);
    let text = reg
        .asset("extra-condensed-bold")
        .ok()
        .or_else(|| reg.system("Saira ExtraCondensed:extrabold"))
        .unwrap_or(label);
    for f in [text, label, small] {
        if *reg.fallbacks(f) != [cjk] {
            reg.set_fallbacks(f, &[cjk]);
        }
    }
    Fonts {
        text,
        label,
        small,
        cjk,
    }
}

/// The popup's width.
pub(crate) fn width(look: &Look) -> f32 {
    look.width.max(MIN_WIDTH)
}

/// Logical pixels per concept unit.
fn scale(look: &Look) -> f32 {
    width(look) / CW
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let s = scale(look);
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX * s, TEXT_LINE * s)
}

/// The centre of the newest transcript line, from the surface top. It is
/// always the box's last line: the transcript is anchored to the bottom.
pub(crate) fn newest_line_centre(look: &Look) -> f32 {
    (TEXT_Y + (MAX_LINES as f32 - 0.5) * TEXT_LINE) * scale(look)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

pub(crate) fn surface(look: &Look) -> Size {
    let s = scale(look);
    Size::new(width(look).ceil(), (CH * s).ceil() + 2.0)
}

fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

/// 0 before `a`, 1 after `b`, linear between.
fn ramp(u: f64, a: f64, b: f64) -> f32 {
    ((u - a) / (b - a)).clamp(0.0, 1.0) as f32
}

fn ease_out(k: f32) -> f32 {
    1.0 - (1.0 - k).powi(3)
}

/// Ease out with a small overshoot, for parts that snap into place.
fn ease_back(k: f32) -> f32 {
    let c = 1.7;
    let x = k - 1.0;
    1.0 + (c + 1.0) * x * x * x + c * x * x
}

fn smooth(k: f32) -> f32 {
    k * k * (3.0 - 2.0 * k)
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn poly(pts: &[(f32, f32)]) -> Contour {
    let pts: Vec<Point> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
    Contour::polyline(&pts, true)
}

/// A closed polyline as an open one that ends where it starts, so a trim
/// draws its last edge too.
fn ring(pts: &[(f32, f32)]) -> Contour {
    let mut pts: Vec<Point> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
    pts.push(pts[0]);
    Contour::polyline(&pts, false)
}

/// The part of `pts`'s polygon between `x0` and `x1` (Sutherland and Hodgman).
fn clip_x(pts: &[(f32, f32)], x0: f32, x1: f32) -> Vec<(f32, f32)> {
    let cut = |pts: Vec<(f32, f32)>, x: f32, keep_right: bool| {
        let inside = |p: (f32, f32)| if keep_right { p.0 >= x } else { p.0 <= x };
        let mut out = Vec::new();
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if inside(a) {
                out.push(a);
            }
            if inside(a) != inside(b) {
                let k = (x - a.0) / (b.0 - a.0);
                out.push((x, a.1 + (b.1 - a.1) * k));
            }
        }
        out
    };
    cut(cut(pts.to_vec(), x0, true), x1, false)
}

/// A small deterministic hash to 0..1, for bar heights that stay put.
fn noise(i: i64) -> f32 {
    let mut x = i as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// R2-23-a: particle `i` of a word at `age` seconds: where it is and its
/// alpha, or none once it has dispersed. It flows from `from` to `to`
/// along a curl, then drifts off. `seed` picks its curl and drift; `k` is
/// the concept-to-pixel scale.
fn ink_dot(age: f64, from: Point, to: Point, seed: i64, k: f32) -> Option<(Point, f32)> {
    if age < 0.0 || age >= INK_IN + INK_OUT {
        return None;
    }
    let (a, b) = (noise(seed) - 0.5, noise(seed + 7919) - 0.5);
    if age < INK_IN {
        let u = (age / INK_IN) as f32;
        let e = 1.0 - (1.0 - u) * (1.0 - u);
        // A curl off the straight path that closes at both ends.
        let bend = INK_CURL * k * (std::f32::consts::PI * e).sin();
        let p = Point::new(
            from.x + (to.x - from.x) * e + 0.3 * bend * b,
            from.y + (to.y - from.y) * e + bend * a,
        );
        return Some((p, INK_ALPHA * smooth((u / 0.25).min(1.0))));
    }
    let v = ((age - INK_IN) / INK_OUT) as f32;
    let drift = INK_DRIFT * k * ease_out(v);
    let p = Point::new(to.x + drift * b * 2.0, to.y + drift * a * 2.0);
    Some((p, INK_ALPHA * (1.0 - v).powf(1.5)))
}

/// R2-23-b: where the band's middle is across the panel at `age`, 0 at the
/// left edge to 1 at the right, and its alpha; none once it has crossed.
fn sweep(age: f64) -> Option<(f32, f32)> {
    if !(0.0..SWEEP_SECS).contains(&age) {
        return None;
    }
    let u = (age / SWEEP_SECS) as f32;
    Some((smooth(u), SWEEP_ALPHA * (std::f32::consts::PI * u).sin()))
}

/// R2-23-c: how much of the lift the newest word has at `age`: up in
/// 120 ms, settled to none at `GRID_SECS`.
fn grid_lift(age: f64) -> f32 {
    if age <= 0.0 || age >= GRID_SECS {
        return 0.0;
    }
    let up = smooth((age / 0.12).min(1.0) as f32);
    let down = 1.0 - smooth(((age - 0.12) / (GRID_SECS - 0.12)).max(0.0) as f32);
    up * down
}

/// R2-23-b: the glass slab's outline, 4 units inside the text panel's
/// border (concept units, clockwise on screen).
const SLAB: [(f32, f32); 8] = [
    (97.6, 304.0),
    (908.4, 304.0),
    (930.0, 325.6),
    (930.0, 489.4),
    (889.4, 530.0),
    (151.6, 530.0),
    (76.0, 454.4),
    (76.0, 325.6),
];

/// R2-23-b: `poly` clipped to the convex polygon `clip`, which runs
/// clockwise on screen (Sutherland-Hodgman).
fn clip_convex(poly: &[(f32, f32)], clip: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut out = poly.to_vec();
    for (i, &a) in clip.iter().enumerate() {
        let b = clip[(i + 1) % clip.len()];
        // Inside is on the right of a to b, with y down.
        let side = |p: (f32, f32)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        let input = std::mem::take(&mut out);
        for (j, &p) in input.iter().enumerate() {
            let q = input[(j + 1) % input.len()];
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let k = sp / (sp - sq);
                out.push((p.0 + (q.0 - p.0) * k, p.1 + (q.1 - p.1) * k));
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// R2-23-c: the text panel's inside at concept height `y`, left and right,
/// `inset` in from its border.
fn panel_span(y: f32, inset: f32) -> (f32, f32) {
    let left = if y < 324.0 {
        96.0 - (y - 300.0)
    } else if y < 456.0 {
        72.0
    } else {
        72.0 + (y - 456.0)
    };
    let right = if y < 324.0 {
        910.0 + (y - 300.0)
    } else if y < 490.0 {
        934.0
    } else {
        934.0 - (y - 490.0) * 42.0 / 44.0
    };
    (left + inset, right - inset)
}

/// MO-53: where the text panel's border snaps, along it from its top left
/// corner (concept units): half way round the top right corner.
const BREAK_AT: f32 = 831.0;
/// MO-53: the stage label's top and its right end at most, above the text
/// panel's border and left of the wave panel's right chevron and slash.
const BREAK_LABEL_Y: f32 = 268.0;
const BREAK_LABEL_RIGHT: f32 = 888.0;

/// The transcript clip's margin on the sides, room for a line that starts
/// with "…", and at the bottom (concept units).
const TEXT_CLIP_SIDE: f32 = 30.0;
const TEXT_CLIP_MARGIN: f32 = 8.0;

// The frame, in concept units.
const OUTER: [(f32, f32); 12] = [
    (52.0, 112.0),
    (928.0, 112.0),
    (972.0, 196.0),
    (972.0, 228.0),
    (950.0, 296.0),
    (950.0, 506.0),
    (870.0, 612.0),
    (138.0, 612.0),
    (48.0, 522.0),
    (48.0, 296.0),
    (12.0, 228.0),
    (12.0, 196.0),
];
const TEXT_PANEL: [(f32, f32); 8] = [
    (96.0, 300.0),
    (910.0, 300.0),
    (934.0, 324.0),
    (934.0, 490.0),
    (892.0, 534.0),
    (150.0, 534.0),
    (72.0, 456.0),
    (72.0, 324.0),
];
const EMBLEM: [(f32, f32); 6] = [
    (78.0, 80.0),
    (122.0, 8.0),
    (222.0, 8.0),
    (266.0, 80.0),
    (222.0, 152.0),
    (122.0, 152.0),
];
const TITLE_BAR: [(f32, f32); 7] = [
    (266.0, 54.0),
    (296.0, 18.0),
    (688.0, 18.0),
    (718.0, 48.0),
    (782.0, 48.0),
    (752.0, 92.0),
    (266.0, 92.0),
];
const BADGE: [(f32, f32); 6] = [
    (785.0, 72.0),
    (818.0, 14.0),
    (898.0, 14.0),
    (932.0, 72.0),
    (898.0, 130.0),
    (818.0, 130.0),
];
const PILL: [(f32, f32); 6] = [
    (290.0, 575.0),
    (314.0, 546.0),
    (666.0, 546.0),
    (690.0, 575.0),
    (666.0, 604.0),
    (314.0, 604.0),
];
const MIC_HEX: [(f32, f32); 6] = [
    (304.0, 575.0),
    (318.0, 553.0),
    (352.0, 553.0),
    (366.0, 575.0),
    (352.0, 597.0),
    (318.0, 597.0),
];

/// A band of hazard stripes: parallelograms leaning by `slant`, clipped to
/// the band.
#[derive(Clone, Copy)]
struct Stripes {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    pitch: f32,
    w: f32,
    slant: f32,
    /// When the first stripe appears, from the popup opening.
    at: f64,
}

const STRIPES: [Stripes; 4] = [
    Stripes {
        x0: 632.0,
        x1: 760.0,
        y0: 62.0,
        y1: 82.0,
        pitch: 26.0,
        w: 15.0,
        slant: 10.0,
        at: 0.34,
    },
    Stripes {
        x0: 122.0,
        x1: 242.0,
        y0: 546.0,
        y1: 560.0,
        pitch: 20.0,
        w: 11.0,
        slant: -7.0,
        at: 0.40,
    },
    Stripes {
        x0: 748.0,
        x1: 800.0,
        y0: 562.0,
        y1: 582.0,
        pitch: 24.0,
        w: 14.0,
        slant: 10.0,
        at: 0.44,
    },
    Stripes {
        x0: 813.0,
        x1: 903.0,
        y0: 112.0,
        y1: 122.0,
        pitch: 23.0,
        w: 13.0,
        slant: -5.0,
        at: 0.38,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Loading,
    Listening,
    Finishing,
    Done,
    Cancelled,
}

impl Phase {
    /// Kanji and English for the badge.
    fn badge(self) -> (&'static str, &'static str) {
        match self {
            Phase::Loading => ("準備中", "STANDBY"),
            Phase::Listening => ("録音中", "LISTENING"),
            Phase::Finishing => ("処理中", "FINISHING"),
            Phase::Done => ("完了", "COMPLETE"),
            Phase::Cancelled => ("中止", "ABORTED"),
        }
    }

    fn ended(self) -> bool {
        matches!(self, Phase::Done | Phase::Cancelled)
    }
}

/// A run of glyphs placed by the canvas: it types in from `start`, a glyph
/// every `per_char` seconds, and with `track` its letters start that far
/// apart and close up.
#[derive(Clone)]
struct Label {
    text: Arc<ShapedText>,
    font: u32,
    px: f32,
    /// Top left of the line box.
    x: f32,
    y: f32,
    color: Color,
    start: f64,
    per_char: f32,
    track: f32,
    /// A solid block runs ahead of the letters as they type in.
    cursor: bool,
}

impl Label {
    fn width(&self) -> f32 {
        self.text.size.w
    }

    /// `red` turns it toward the cancelled red, 0 to 1.
    fn draw(&self, p: &mut Painter, t: f64, red: f32) {
        let n = self.text.glyphs.len();
        if t < self.start {
            return;
        }
        for (i, g) in self.text.glyphs.iter().enumerate() {
            let ti = self.start + (i as f32 * self.per_char) as f64;
            if t < ti {
                break;
            }
            let a = ramp(t, ti, ti + 0.05);
            let dx = if self.track > 0.0 {
                let k = ease_out(ramp(
                    t,
                    self.start + 0.02 * i as f64,
                    self.start + 0.02 * i as f64 + 0.4,
                ));
                self.track * (1.0 - k) * (i as f32 - (n as f32 - 1.0) / 2.0)
            } else {
                0.0
            };
            p.glyph(GlyphInstance {
                xform: vo::scene::display::GlyphXform::IDENTITY,
                pos: pt(self.x + g.pos.x + dx, self.y + g.pos.y),
                size: self.px,
                font: self.font,
                glyph: g.glyph,
                word: NO_WORD,
                color: mix(self.color, hex(RED), red).fade(a),
            });
        }
        let typed = self.start + (n as f32 * self.per_char) as f64;
        if self.cursor && self.px < 20.0 && self.per_char > 0.0 && t < typed + 0.08 {
            let i = (((t - self.start) / self.per_char as f64) as usize).min(n);
            let x = self.text.glyphs.get(i).map_or(self.width(), |g| g.pos.x);
            p.shape(ShapeInstance::rrect(
                RRect::new(
                    Rect::new(
                        self.x + x,
                        self.y + self.px * 0.12,
                        self.px * 0.5,
                        self.px * 0.9,
                    ),
                    0.0,
                ),
                Paint::solid(mix(hex(YELLOW), hex(RED), red)),
            ));
        }
    }
}

/// Everything the canvas needs, taken when the view is built.
#[derive(Clone)]
struct Snap {
    shown_at: f64,
    phase: Phase,
    /// When the phase began.
    phase_at: f64,
    finish_at: Option<f64>,
    /// When the stripes stop: done or cancelled.
    end_at: Option<f64>,
    levels: Arc<[f32]>,
    newest_at: f64,
    /// The bars' clock during a rewind cancel (MO-50).
    scrub: endings::Scrub,
    window: f32,
    fraction: Option<f32>,
    labels: Vec<Label>,
    /// The badge's words, typed in when the phase begins.
    badge: Vec<Label>,
    /// The status line: icon, state and detail.
    status: Vec<Label>,
    icon: Icon,
    /// When new words came, newest last: each pushes the status plate.
    pushes: Vec<f64>,
    /// MO-53: the text panel's border is drawn snapped by
    /// `paths_broken_frame_at` instead.
    broken: bool,
    /// Reduced motion: the stripes stand still and there is no idle motion.
    reduced: bool,
    /// `BREATHE_PX` in concept units.
    breathe: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Icon {
    Mic,
    Stop,
    Check,
    Cross,
}

fn icon_outline(icon: Icon) -> Arc<Outline> {
    static ICONS: OnceLock<[Arc<Outline>; 4]> = OnceLock::new();
    let all = ICONS.get_or_init(|| {
        let o = |p| Arc::new(Outline::new(&p, 0.05));
        let mut stop = vo::scene::path::kurbo::BezPath::new();
        stop.move_to((7.0, 7.0));
        stop.line_to((17.0, 7.0));
        stop.line_to((17.0, 17.0));
        stop.line_to((7.0, 17.0));
        stop.close_path();
        [o(icons::mic()), o(stop), o(icons::check()), o(icons::x())]
    });
    all[icon as usize].clone()
}

impl Snap {
    fn red(&self, t: f64) -> f32 {
        if self.phase == Phase::Cancelled {
            ease_out(ramp(t, self.phase_at, self.phase_at + 0.15))
        } else {
            0.0
        }
    }

    /// How far the stripes have slid.
    fn shift(&self, t: f64) -> f32 {
        if self.reduced {
            return 0.0;
        }
        let end = self.end_at.unwrap_or(f64::INFINITY).min(t);
        let fin = self.finish_at.unwrap_or(f64::INFINITY).min(end);
        let mut d = (fin - self.shown_at).max(0.0) * 14.0;
        if let Some(f) = self.finish_at {
            d += (end - f).max(0.0) * 42.0;
        }
        d as f32
    }

    /// The idle motion runs: listening, past the opening, motion allowed.
    fn idle(&self, u: f64) -> bool {
        !self.reduced && u > 0.62 && matches!(self.phase, Phase::Listening | Phase::Loading)
    }

    /// The voice level at `t`: each level decays from when it was heard.
    fn level(&self, t: f64) -> f32 {
        let n = self.levels.len();
        let mut l = 0.0f32;
        for k in 0..n.min(60) {
            let i = n - 1 - k;
            let at = self.newest_at - k as f64 / FRAME_HZ as f64;
            if at > t {
                continue;
            }
            let age = (t - at) as f32;
            l = l.max(self.levels[i] * (-age / 0.25).exp());
        }
        l
    }

    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let s = rect.w / CW;
        p.layer(
            Affine::scale(s).then(&Affine::translate(rect.x, rect.y)),
            1.0,
            |p| self.draw_units(t, p),
        );
    }

    fn draw_units(&self, t: f64, p: &mut Painter) {
        let u = t - self.shown_at;
        let red = self.red(t);
        let accent = mix(hex(ORANGE), hex(RED), red);
        // The frame flashes once when the text is done.
        let flash = if self.phase == Phase::Done {
            1.0 + 0.8 * (1.0 - ramp(t, self.phase_at, self.phase_at + 0.35))
        } else {
            1.0
        };
        let line = |w: f32| StrokeStyle::solid(w, accent).glow(1.6 * flash);
        let fill_k = ease_out(ramp(u, 0.12, 0.40));

        // The body: fill, then the outline drawing itself on.
        let outer = ring(&OUTER);
        p.fill(
            [&poly(&OUTER)],
            &StrokeStyle::solid(1.0, hex(FILL).fade(0.94 * fill_k)),
        );
        let k = ease_out(ramp(u, 0.0, 0.36));
        p.stroke(&outer.trim(0.0, k * outer.length()), &line(3.2));

        // The wave panel's grid, crosshairs and chevrons.
        let dim = accent.fade(0.09 * fill_k);
        for x in [245.0, 478.0, 712.0] {
            p.stroke(
                &poly(&[(x, 120.0), (x, 292.0)]),
                &StrokeStyle::solid(1.0, dim),
            );
        }
        for (x, y) in [
            (290.0, 146.0),
            (580.0, 146.0),
            (290.0, 266.0),
            (580.0, 266.0),
            (866.0, 146.0),
        ] {
            let c = accent.fade(0.7 * fill_k);
            p.stroke(
                &poly(&[(x - 6.0, y), (x + 6.0, y)]),
                &StrokeStyle::solid(1.2, c),
            );
            p.stroke(
                &poly(&[(x, y - 6.0), (x, y + 6.0)]),
                &StrokeStyle::solid(1.2, c),
            );
        }
        let chev = ease_out(ramp(u, 0.2, 0.45));
        let open = |pts: &[(f32, f32)]| {
            let pts: Vec<Point> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
            Contour::polyline(&pts, false)
        };
        let l = open(&[(130.0, 140.0), (108.0, 206.0), (130.0, 272.0)]);
        let r = open(&[(898.0, 140.0), (920.0, 206.0), (898.0, 272.0)]);
        p.stroke(
            &l.trim(0.0, chev * l.length()),
            &StrokeStyle::solid(1.5, accent.fade(0.8)),
        );
        p.stroke(
            &r.trim(0.0, chev * r.length()),
            &StrokeStyle::solid(1.5, accent.fade(0.8)),
        );

        // The text panel.
        let panel = ring(&TEXT_PANEL);
        p.fill(
            [&poly(&TEXT_PANEL)],
            &StrokeStyle::solid(1.0, hex(PANEL).fade(fill_k)),
        );
        let k = ease_out(ramp(u, 0.06, 0.42));
        if !self.broken {
            p.stroke(&panel.trim(0.0, k * panel.length()), &line(2.8));
        }

        // Notches, side bars and slashes slide in from outside.
        let slide = (1.0 - ease_back(ramp(u, 0.26, 0.50))) * 24.0;
        let a = ramp(u, 0.26, 0.34);
        let solid = |c: Color| StrokeStyle::solid(1.0, c.fade(a)).glow(3.0);
        p.fill(
            [&poly(&[
                (12.0 - slide, 190.0),
                (30.0 - slide, 212.0),
                (12.0 - slide, 234.0),
            ])],
            &solid(accent),
        );
        p.fill(
            [&poly(&[
                (972.0 + slide, 190.0),
                (954.0 + slide, 212.0),
                (972.0 + slide, 234.0),
            ])],
            &solid(accent),
        );
        // Idle: the bars' facing ends reach toward each other and back.
        let reach = if self.idle(u) {
            let k = (std::f64::consts::TAU * u / BREATHE_SECS).cos() as f32;
            self.breathe * 0.5 * (1.0 - k)
        } else {
            0.0
        };
        for (y0, y1) in [(330.0, 384.0 + reach), (418.0 - reach, 470.0)] {
            p.fill(
                [&poly(&[
                    (44.0 - slide, y0),
                    (52.0 - slide, y0),
                    (52.0 - slide, y1),
                    (44.0 - slide, y1),
                ])],
                &solid(accent),
            );
        }
        for (a0, a1) in [
            ((896.0, 146.0), (926.0, 200.0)),
            ((926.0, 226.0), (896.0, 280.0)),
            ((900.0, 324.0), (922.0, 366.0)),
        ] {
            p.stroke(
                &poly(&[(a0.0 + slide, a0.1), (a1.0 + slide, a1.1)]),
                &StrokeStyle::solid(5.0, accent.fade(a)).glow(3.0),
            );
        }
        p.stroke(
            &poly(&[(86.0 - slide, 410.0), (108.0 - slide, 448.0)]),
            &StrokeStyle::solid(4.0, accent.fade(a)).glow(3.0),
        );

        self.draw_wave(t, u, accent, p);
        self.draw_meter(t, u, accent, p);

        // The plates on top: title bar, emblem and badge.
        let tb = ring(&TITLE_BAR);
        p.fill(
            [&poly(&TITLE_BAR)],
            &StrokeStyle::solid(1.0, hex(FILL).fade(fill_k)),
        );
        let k = ease_out(ramp(u, 0.10, 0.40));
        p.stroke(&tb.trim(0.0, k * tb.length()), &line(3.2));
        self.plate(
            p,
            &EMBLEM,
            (172.0, 80.0),
            ramp(u, 0.18, 0.42),
            accent,
            flash,
        );
        self.plate(p, &BADGE, (858.0, 72.0), ramp(u, 0.24, 0.48), accent, flash);
        // The badge's rule.
        let rk = ease_out(ramp(u, 0.4, 0.6));
        p.stroke(
            &poly(&[(858.0 - 46.0 * rk, 89.0), (858.0 + 46.0 * rk, 89.0)]),
            &StrokeStyle::solid(1.0, accent.fade(0.8)),
        );
        // The readout's dot blinks while recording.
        let on = match self.phase {
            Phase::Listening | Phase::Loading => (u * 2.0).fract() < 0.5,
            _ => true,
        };
        if on && u > 0.5 {
            p.shape(ShapeInstance::circle(
                pt(826.0, 101.0),
                3.0,
                Paint::solid(accent),
            ));
        }

        // The status band: pill, joints and the mic hexagon. A new word
        // pushes the plate along its rails (MO-70).
        let push = Affine::translate(self.push(t), 0.0);
        let k = ease_out(ramp(u, 0.16, 0.46));
        p.layer(push, 1.0, |p| {
            let pill = ring(&PILL);
            p.fill(
                [&poly(&PILL)],
                &StrokeStyle::solid(1.0, hex(FILL).fade(fill_k)),
            );
            p.stroke(&pill.trim(0.0, k * pill.length()), &line(2.8));
            let mh = ring(&MIC_HEX);
            p.stroke(
                &mh.trim(0.0, k * mh.length()),
                &StrokeStyle::solid(1.2, accent.fade(0.7)),
            );
        });
        for j in [
            open(&[(110.0, 542.0), (270.0, 542.0), (290.0, 575.0)]),
            open(&[(690.0, 575.0), (710.0, 542.0), (884.0, 542.0)]),
        ] {
            p.stroke(
                &j.trim(0.0, k * j.length()),
                &StrokeStyle::solid(1.5, accent.fade(0.85)),
            );
        }

        // Hazard stripes.
        let shift = self.shift(t);
        for (b, band) in STRIPES.iter().enumerate() {
            let n = ((band.x1 - band.x0) / band.pitch).ceil() as i32 + 2;
            let off = (shift * if b % 2 == 0 { 1.0 } else { -1.0 }).rem_euclid(band.pitch);
            for i in -1..n {
                let order = (i + 1) as f64;
                let a = ramp(u, band.at + 0.04 * order, band.at + 0.04 * order + 0.05);
                if a <= 0.0 {
                    continue;
                }
                let x = band.x0 + i as f32 * band.pitch + off;
                let quad = [
                    (x + band.slant, band.y0),
                    (x + band.slant + band.w, band.y0),
                    (x + band.w, band.y1),
                    (x, band.y1),
                ];
                let clipped = clip_x(&quad, band.x0, band.x1);
                if clipped.len() >= 3 {
                    p.fill([&poly(&clipped)], &StrokeStyle::solid(1.0, accent.fade(a)));
                }
            }
        }

        for l in &self.labels {
            l.draw(p, t, red);
        }
        for l in &self.badge {
            l.draw(p, t, red);
        }
        p.layer(push, 1.0, |p| {
            for l in &self.status {
                l.draw(p, t, red);
            }
            self.draw_icon(t, u, accent, p);
        });
    }

    /// How far the status plate is pushed along its rails at `t`, in
    /// concept units: each new word knocks it `PUSH` to the right, two links
    /// down the chain from the caret, and it springs back.
    fn push(&self, t: f64) -> f32 {
        let d: f32 = self
            .pushes
            .iter()
            .map(|&at| sounding::knock(t - at - 2.0 * LINK, 0.035))
            .sum();
        PUSH * d.min(1.6)
    }

    /// A hexagon plate that snaps in around `c`.
    fn plate(
        &self,
        p: &mut Painter,
        pts: &[(f32, f32)],
        c: (f32, f32),
        k: f32,
        accent: Color,
        flash: f32,
    ) {
        if k <= 0.0 {
            return;
        }
        let sc = 1.18 - 0.18 * ease_back(k);
        let m = Affine::translate(-c.0, -c.1)
            .then(&Affine::scale(sc))
            .then(&Affine::translate(c.0, c.1));
        p.layer(m, ease_out(ramp(k as f64, 0.0, 0.4)), |p| {
            p.fill([&poly(pts)], &StrokeStyle::solid(1.0, hex(FILL)));
            p.stroke(
                &ring(pts),
                &StrokeStyle::solid(3.2, accent).glow(1.6 * flash),
            );
        });
    }

    fn draw_wave(&self, t: f64, u: f64, accent: Color, p: &mut Painter) {
        let w = WAVE_X1 - WAVE_X0;
        let amp = ease_out(ramp(u, 0.30, 0.70));
        match self.phase {
            Phase::Loading => {
                let k = ramp(u, 0.3, 0.5);
                let mut x = WAVE_X0;
                while x <= WAVE_X1 {
                    p.shape(ShapeInstance::circle(
                        pt(x, WAVE_CY),
                        1.3,
                        Paint::solid(accent.fade(0.7 * k)),
                    ));
                    x += 8.0;
                }
                let y = WAVE_CY + 34.0;
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(WAVE_X0 + 20.0, y, w - 40.0, 4.0), 0.0),
                    Paint::solid(accent.fade(0.18 * k)),
                ));
                let (a, b) = match self.fraction {
                    Some(f) => (0.0, f.clamp(0.0, 1.0)),
                    None => {
                        let c = ((t - self.shown_at) / 1.4).fract() as f32;
                        ((c * 1.3 - 0.3).max(0.0), (c * 1.3).min(1.0))
                    }
                };
                let span = w - 40.0;
                p.shape(ShapeInstance::rrect(
                    RRect::new(
                        Rect::new(WAVE_X0 + 20.0 + a * span, y, (b - a) * span, 4.0),
                        0.0,
                    ),
                    Paint::solid(accent.fade(k)),
                ));
                return;
            }
            Phase::Cancelled => {
                let mut x = WAVE_X0;
                while x <= WAVE_X1 {
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(x, WAVE_CY - 0.8, 5.0, 1.6), 0.0),
                        Paint::solid(accent.fade(0.8)),
                    ));
                    x += 9.0;
                }
                return;
            }
            _ => {}
        }
        // Done: the bars fall to dots, then a bright line opens from the centre.
        let settle = if self.phase == Phase::Done {
            1.0 - smooth(ramp(t, self.phase_at, self.phase_at + 0.4))
        } else {
            1.0
        };
        let frozen = matches!(self.phase, Phase::Finishing | Phase::Done);
        let slot = self.window / (w / BAR_PITCH);
        let tc = self.scrub.map(t).min(self.newest_at);
        let n = self.levels.len();
        let last = (tc / slot as f64).floor() as i64;
        let count = (w / BAR_PITCH) as i64;
        for k in 0..count {
            let j = last - k;
            let ts = j as f64 * slot as f64;
            let x = WAVE_X1 - ((tc - ts) as f32 / self.window) * w;
            if x < WAVE_X0 {
                break;
            }
            // The loudest level in the slot.
            let mut l = 0.0f32;
            for q in 0..3 {
                let at = ts + q as f64 * slot as f64 / 3.0;
                let back = ((self.newest_at - at) * FRAME_HZ as f64).round();
                if back >= 0.0 && (back as usize) < n {
                    l = l.max(self.levels[n - 1 - back as usize]);
                }
            }
            let xn = (x - WAVE_X0) / w;
            let taper = (std::f32::consts::PI * xn).sin().max(0.0).powf(0.7);
            let spike = 0.2 + 0.8 * noise(j).powf(1.4);
            let f = (1.4 * l.min(1.2).powf(0.6) * spike * taper * amp * settle).min(1.0);
            let h = 1.4 + WAVE_HALF * f;
            let mut core = mix(hex(DEEP), hex(YELLOW), f.powf(1.1));
            let mut tip = mix(hex(0xc42800), hex(ORANGE), f);
            if self.phase == Phase::Finishing {
                // Frozen and dim, brighter behind the progress.
                let behind = self.fraction.is_some_and(|fr| xn <= fr);
                let k = if behind { 0.8 } else { 0.42 };
                core = core.fade(k);
                tip = tip.fade(k);
            }
            let r = Rect::new(x - BAR_W / 2.0, WAVE_CY - h, BAR_W, 2.0 * h);
            if f > 0.08 && !frozen {
                p.shape(ShapeInstance::rrect(
                    RRect::new(r, BAR_W / 2.0),
                    Paint::glow(core, 3.0, 0.25 * f),
                ));
            }
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(r.x, r.y, r.w, h), BAR_W / 2.0),
                Paint::linear(pt(x, WAVE_CY - h), tip, pt(x, WAVE_CY), core),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(r.x, WAVE_CY, r.w, h), BAR_W / 2.0),
                Paint::linear(pt(x, WAVE_CY), core, pt(x, WAVE_CY + h), tip),
            ));
        }
        if self.phase == Phase::Finishing {
            let k = ((t - self.phase_at) / 1.2).fract() as f32;
            let x = WAVE_X0 + k * w;
            let r = Rect::new(x - 1.0, WAVE_CY - WAVE_HALF, 2.0, 2.0 * WAVE_HALF);
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.0),
                Paint::glow(hex(YELLOW), 3.0, 0.6),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.0),
                Paint::solid(hex(YELLOW)),
            ));
        }
        if self.phase == Phase::Done {
            let k = ease_out(ramp(t, self.phase_at + 0.3, self.phase_at + 0.6));
            let half = k * w / 2.0;
            let c = (WAVE_X0 + WAVE_X1) / 2.0;
            let r = Rect::new(c - half, WAVE_CY - 1.5, 2.0 * half, 3.0);
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.5),
                Paint::glow(hex(DEEP), 5.0, 0.8),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.5),
                Paint::solid(hex(0xff9a30)),
            ));
        }
    }

    fn draw_meter(&self, t: f64, u: f64, accent: Color, p: &mut Painter) {
        let a = ramp(u, 0.3, 0.4);
        for x in [56.0, 96.0] {
            p.stroke(
                &poly(&[(x, 156.0), (x, 278.0)]),
                &StrokeStyle::solid(1.2, accent.fade(0.6 * a)),
            );
        }
        let lit = if u < 0.62 {
            (ramp(u, 0.40, 0.60) * 10.0).floor() as i32
        } else if self.phase.ended() || self.phase == Phase::Loading {
            0
        } else {
            (14.0 * self.level(t).min(1.2).powf(0.6)).round() as i32
        }
        .min(10);
        for i in 0..10 {
            let y = 272.0 - (i + 1) as f32 * 12.0 + 3.0;
            let on = i < lit;
            let c = if on {
                let c = mix(hex(YELLOW), hex(DEEP), i as f32 / 9.0);
                if i == lit - 1 && i > 6 {
                    mix(c, Color::WHITE, 0.5)
                } else {
                    c
                }
            } else if i == lit && self.idle(u) {
                // Idle: the cell above the reading jitters like a needle.
                let step = (t * JITTER_HZ).floor() as i64;
                accent.fade(0.18 + JITTER_INK * noise(step))
            } else {
                accent.fade(0.18)
            };
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(64.0, y, 22.0, 9.0), 0.0),
                Paint::solid(c.fade(a)),
            ));
        }
    }

    /// The status icon: the new one pops in as the old one shrinks away.
    fn draw_icon(&self, t: f64, u: f64, accent: Color, p: &mut Painter) {
        let k = if self.icon == Icon::Mic {
            ramp(u, 0.35, 0.6)
        } else {
            ramp(t, self.phase_at, self.phase_at + 0.22)
        };
        if k <= 0.0 {
            return;
        }
        let size = 36.0 * (0.6 + 0.4 * ease_back(k));
        let c = (335.0, 575.0);
        let m = Affine::translate(-12.0, -12.0)
            .then(&Affine::scale(size / 24.0))
            .then(&Affine::translate(c.0, c.1));
        let color = if self.phase == Phase::Loading {
            accent.fade(0.5)
        } else {
            accent
        };
        let o = icon_outline(self.icon);
        p.layer(m, ease_out(k), |p| {
            p.outline(&o, &StrokeStyle::solid(2.8, color).glow(1.5));
        });
    }
}

/// A strike line through each line of the transcript, drawn on when a
/// dictation is cancelled.
fn draw_strikes(t: f64, at: f64, widths: &[f32], rect: Rect, p: &mut Painter) {
    let s = rect.w / TEXT_W;
    for (i, w) in widths.iter().enumerate() {
        let k = ease_out(ramp(t, at + 0.04 * i as f64, at + 0.04 * i as f64 + 0.15));
        // The text sits at the bottom of its box.
        let row = (MAX_LINES as usize - widths.len().min(MAX_LINES as usize) + i) as f32;
        let y = rect.y + (row + 0.58) * TEXT_LINE * s;
        let x0 = rect.x - 6.0 * s;
        let r = Rect::new(x0, y - 2.0 * s, (w + 12.0 * s) * k, 4.0 * s);
        p.shape(ShapeInstance::rrect(
            RRect::new(r, 0.0),
            Paint::glow(hex(RED), 1.5 * s, 0.4),
        ));
        p.shape(ShapeInstance::rrect(
            RRect::new(r, 0.0),
            Paint::solid(hex(RED)),
        ));
    }
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

/// Shaped labels by text, font and size in 1/64 units.
type ShapedCache = HashMap<(String, u32, u32), Arc<ShapedText>>;

/// Shaped labels, kept: the view is built for every audio batch. Font ids
/// belong to a font table, so each table has its own cache (tests each load
/// their own table).
fn shaped(measure: &vo::ShapedLayout, text: &str, font: u32, px: f32) -> Arc<ShapedText> {
    static CACHES: Mutex<Vec<(FontRegistry, ShapedCache)>> = Mutex::new(Vec::new());
    let key = (text.to_string(), font, (px * 64.0) as u32);
    let mut caches = CACHES.lock().unwrap();
    let same = |(f, _): &(FontRegistry, ShapedCache)| f.same_table(measure.fonts());
    let i = match caches.iter().position(same) {
        Some(i) => i,
        None => {
            if caches.len() >= 4 {
                caches.remove(0);
            }
            caches.push((measure.fonts().clone(), ShapedCache::new()));
            caches.len() - 1
        }
    };
    let cache = &mut caches[i].1;
    if let Some(s) = cache.get(&key) {
        return s.clone();
    }
    if cache.len() > 512 {
        cache.clear();
    }
    let style = TextStyle::new(font, px, (px * 1.25).ceil());
    let s = Arc::new(measure.layout(text, &style, None));
    cache.insert(key, s.clone());
    s
}

impl Popup {
    fn mecha_phase(&self, status: &StatusLine) -> (Phase, f64) {
        let e = &self.extra;
        if self.ended() && e.finish_at.is_none() {
            return (Phase::Cancelled, self.last_audio.unwrap_or(self.shown_at));
        }
        // The final text has landed: done, even while it is being typed.
        if let Some(f) = e.final_at {
            return (Phase::Done, f);
        }
        if self.ended() {
            return (Phase::Done, e.finish_at.unwrap_or(self.shown_at));
        }
        if status.loading {
            return (Phase::Loading, self.shown_at);
        }
        if let Some(f) = e.finish_at {
            return (Phase::Finishing, f);
        }
        (Phase::Listening, self.shown_at)
    }

    /// MO-53: on a failure the text panel's border snaps at its top right
    /// corner, the parted ends lit yellow, with the stage that failed above
    /// it, left of the break and clear of the outer frame.
    fn mecha_broken_frame(&self, size: Size) -> Option<Element<Msg>> {
        if !self.paths_broken() {
            return None;
        }
        let s = scale(&self.look);
        let border = ring(&TEXT_PANEL.map(|(x, y)| (x * s, y * s)));
        let pen = glyph_paths::BrokenPen {
            width: 2.8 * s,
            glow: 1.6,
            line: hex(ORANGE),
            accent: hex(YELLOW),
            label: TextStyle::new(fonts(&self.look.fonts).label, 13.0, 16.0).tracking(0.6),
            label_y: BREAK_LABEL_Y * s,
        };
        let right = BREAK_LABEL_RIGHT * s;
        self.paths_broken_frame_within(&border, BREAK_AT * s, size.w, size.h, pen, right)
    }

    /// MO-50: a cancel takes the words back into the caret, drawn over the
    /// hidden text at the bottom of its box.
    fn mecha_rewind(&self, color: Color, s: f32) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let l = &self.look;
        let (style, w) = (text_style(l), text_width(l));
        let h = MAX_LINES as f32 * TEXT_LINE * s;
        let laid = self.lay_words(style, w);
        let dy = h - laid.text.size.h;
        let ink = endings::Ink {
            text: color,
            mark: hex(YELLOW),
            faint: hex(DEEP),
        };
        self.rewind_mark("rewind", laid.shifted(0.0, dy), ink, Size::new(w, h))
    }

    pub(crate) fn mecha_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        let l = &self.look;
        let s = scale(l);
        let size = surface(l);
        let status = self.status();
        let (phase, phase_at) = self.mecha_phase(&status);
        let f = fonts(&l.fonts);
        let m = &self.measure;
        let label =
            |text: &str, font: u32, px: f32, x: f32, y: f32, color: u32, start: f64| Label {
                text: shaped(m, text, font, px),
                font,
                px,
                x,
                y,
                color: hex(color),
                start: self.shown_at + start,
                per_char: 0.015,
                track: 0.0,
                cursor: true,
            };
        let centred = |mut lb: Label| {
            lb.x -= lb.width() / 2.0;
            lb
        };
        let mut labels = vec![
            centred(label("VX", f.text, 66.0, 172.0, -2.0, ORANGE, 0.30)),
            centred(label(
                "VOICE UNIT 01",
                f.label,
                12.5,
                172.0,
                66.0,
                ORANGE,
                0.36,
            )),
            label("SPEECH TO TEXT", f.small, 9.0, 112.0, 88.0, ORANGE, 0.42),
            label("ALL CHANNELS OPEN", f.small, 9.0, 112.0, 99.0, ORANGE, 0.46),
            label("UNIT READY.", f.small, 9.0, 112.0, 110.0, ORANGE, 0.50),
            Label {
                track: 26.0,
                per_char: 0.02,
                cursor: false,
                ..label("音声入力", f.cjk, 42.0, 300.0, 22.0, ORANGE, 0.22)
            },
            label("VOICE DICTATION", f.label, 16.0, 478.0, 33.0, ORANGE, 0.32),
            label(
                "REAL-TIME TRANSCRIPTION",
                f.small,
                10.5,
                478.0,
                58.0,
                ORANGE,
                0.40,
            ),
            label("MAX", f.label, 12.0, 61.0, 132.0, ORANGE, 0.42),
            label("MIN", f.label, 12.0, 62.0, 280.0, ORANGE, 0.46),
            label("作戦継続中", f.cjk, 15.0, 124.0, 562.0, 0xff5a2a, 0.48),
            label(
                "CONTINUOUS OPERATION",
                f.small,
                9.5,
                124.0,
                585.0,
                ORANGE,
                0.52,
            ),
            label("VOX-01", f.label, 18.0, 814.0, 556.0, ORANGE, 0.50),
        ];
        if phase == Phase::Loading && self.shown.is_empty() {
            labels.push(label(
                "AWAITING VOICE INPUT",
                f.text,
                24.0,
                TEXT_X,
                TEXT_Y,
                0x9a4a14,
                0.5,
            ));
        }
        let count = format!("{:03} WORDS", self.word_count);
        labels.push(Label {
            per_char: 0.0,
            cursor: false,
            ..label(&count, f.small, 10.0, 814.0, 582.0, ORANGE, 0.56)
        });
        let readout = format!("REC {}", self.clock);
        labels.push(Label {
            per_char: 0.0,
            cursor: false,
            ..label(&readout, f.small, 9.0, 834.0, 94.0, ORANGE, 0.56)
        });

        // The badge types its words in when the phase begins.
        let (kanji, english) = match self.ending() {
            Some(Ending::Failed) => ("中止", "FAILED"),
            _ => phase.badge(),
        };
        let badge_at = if phase == Phase::Listening || phase == Phase::Loading {
            0.40
        } else {
            phase_at - self.shown_at
        };
        let mut badge = vec![
            centred(Label {
                per_char: 0.04,
                cursor: false,
                ..label(kanji, f.cjk, 24.0, 858.0, 18.0, ORANGE, badge_at)
            }),
            centred(label(
                english,
                f.label,
                14.0,
                858.0,
                52.0,
                ORANGE,
                badge_at + 0.05,
            )),
        ];
        if phase == Phase::Cancelled {
            for b in &mut badge {
                b.color = hex(RED);
            }
        }

        // The status line.
        let (state, detail) = match phase {
            _ if self.ending() == Some(Ending::Failed) => {
                ("Failed".to_string(), "nothing was typed".to_string())
            }
            _ if self.ending() == Some(Ending::Empty) => {
                (Ending::Empty.label().to_string(), String::new())
            }
            Phase::Cancelled => ("Cancelled".to_string(), String::new()),
            Phase::Done => {
                let app = self.files.live.as_ref().and_then(|d| d.target.as_ref());
                match app {
                    Some(a) => (format!("Typed into {}", a.app), String::new()),
                    None => ("Done".to_string(), "text typed".to_string()),
                }
            }
            _ => (
                status.state.clone(),
                status
                    .detail
                    .trim_start_matches([' ', '·'])
                    .trim()
                    .to_string(),
            ),
        };
        let status_at = if phase == Phase::Listening {
            0.44
        } else {
            phase_at - self.shown_at
        };
        let state_l = label(&state, f.label, 20.0, 382.0, 561.0, ORANGE, status_at);
        let mut status_labels = vec![];
        let mut x = 382.0 + state_l.width();
        status_labels.push(state_l);
        if !detail.is_empty() {
            status_labels.push(label(
                "·",
                f.label,
                20.0,
                x + 12.0,
                561.0,
                ORANGE,
                status_at + 0.1,
            ));
            x += 34.0;
            status_labels.push(label(
                &detail,
                f.label,
                16.0,
                x,
                564.0,
                HINT,
                status_at + 0.12,
            ));
        }
        let icon = match phase {
            Phase::Loading | Phase::Listening => Icon::Mic,
            Phase::Finishing => Icon::Stop,
            Phase::Done => Icon::Check,
            Phase::Cancelled => Icon::Cross,
        };

        let chain = self.mecha_chain(phase);
        let end_at = phase.ended().then_some(phase_at);
        let snap = Snap {
            shown_at: self.shown_at,
            phase,
            phase_at,
            finish_at: self.extra.finish_at,
            end_at,
            levels: self.wave.clone(),
            newest_at: self.newest_at,
            scrub: self.scrub(),
            window: l.window as f32,
            fraction: self.files.fraction.map(|f| f as f32),
            labels,
            badge,
            status: status_labels,
            icon,
            pushes: chain.pushes(),
            broken: self.paths_broken(),
            reduced: kit::reduced() || l.on("reduced-motion"),
            breathe: BREATHE_PX / s,
        };
        // The canvas keeps drawing while anything on it moves: the stripes
        // and the readout's dot until the dictation ends.
        let until = match end_at {
            Some(e) => e + 1.0,
            None => f64::INFINITY,
        };
        // Ink past the box: the side notches slide in from 12 units outside,
        // plus stroke and glow. Top and bottom only carry glow.
        let canvas = Element::canvas("frame", until, move |t, rect, p| snap.draw(t, rect, p))
            .size(size.w, size.h)
            .declare_reach(vo::scene::Insets::new(18.0 * s, 6.0 * s, 18.0 * s, 6.0 * s));

        let text_color = if phase == Phase::Cancelled {
            mix(hex(TEXT), hex(RED), 0.35).fade(0.55)
        } else {
            hex(TEXT)
        };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(text_style(l))
            .color(text_color)
            .text_halo(hex(DEEP).fade(0.25), 2.0 * s)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            // The block grows upward from the bottom of the box as lines wrap.
            // It jumps there, so the newest line arrives on the box's last line
            // and stays (RD-09).
            .motion(Prop::Position, Motion::Snap)
            .word_enter(self.ghost_enter(Transition::fade().offset(10.0 * s, 0.0)));
        // Anchored to the bottom, so the newest line always sits on the
        // last line of the box.
        let box_h = MAX_LINES as f32 * TEXT_LINE * s;
        let text = Element::column("text")
            .justify(JustifyContent::FLEX_END)
            .width(text_width(l))
            .height(box_h)
            .child(words.rewind_hide(self.rewinding()))
            .children(self.mecha_rewind(text_color, s));
        // The ghost tail starts where the block's first line is now, and has
        // room down to the box's last line only.
        let (style, wrap) = (text_style(l), text_width(l));
        let joined = self.shown.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let rows = ((self.measure.layout(&joined, &style, Some(wrap)).size.h / style.line_height).round() as u32).max(1);
        let tail = model_signals::TailText { style, width: wrap, rows };
        let top = box_h - rows as f32 * style.line_height;
        // Clipped to the text box, so a line folding out at the top on an
        // overflow stays off the panel's border. A margin on the sides and
        // bottom keeps the halo and the word entry whole; none at the top,
        // where the box meets the border.
        let (side, margin) = (TEXT_CLIP_SIDE * s, TEXT_CLIP_MARGIN * s);
        let text = at(
            Element::layer("text-clip")
                .size(text_width(l) + 2.0 * side, box_h + margin)
                .clip()
                .child(at(
                    self.signal_marks_in(text, &self.shown, &tail, (0.0, top), text_color),
                    side,
                    0.0,
                )),
            TEXT_X * s - side,
            TEXT_Y * s,
        );

        let exit = if phase == Phase::Cancelled {
            Transition::fade().scale(0.94)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let mut layer = Element::layer("mecha")
            .size(size.w, size.h)
            .presence(Transition::fade().opacity(0.0), exit)
            .child(at(canvas, 0.0, 0.0))
            .children(self.mecha_broken_frame(size))
            .children(self.mecha_behind_text(phase))
            .child(at(chain.element(), TEXT_X * s, TEXT_Y * s))
            .child(text)
            .children(self.mecha_sweep(phase));
        if phase == Phase::Cancelled {
            // Strike lines over the words, as wide as each line.
            let style = text_style(l);
            let shaped = m.layout(
                &self
                    .shown
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                &style,
                Some(text_width(l)),
            );
            let mut widths = vec![0.0f32; MAX_LINES as usize];
            for g in &shaped.glyphs {
                let line = ((g.pos.y / style.line_height).floor() as usize).min(widths.len() - 1);
                widths[line] = widths[line].max(g.pos.x + 0.5 * style.size);
            }
            widths.retain(|w| *w > 0.0);
            let at_t = phase_at;
            let strikes = Element::canvas("strikes", at_t + 0.5, move |t, rect, p| {
                draw_strikes(t, at_t, &widths, rect, p)
            })
            .size(text_width(l), MAX_LINES as f32 * TEXT_LINE * s);
            layer = layer.child(at(strikes, TEXT_X * s, TEXT_Y * s));
        }
        root.child(layer)
    }
}

/// The hydraulic chain (MO-70) and the onset joint (MO-73), drawn behind the
/// transcript in the text box's space: the caret block on its foot, the
/// plates that slide out from under revised words, and the baseline ticks
/// that onsets leave under the words they started.
#[derive(Clone, Default)]
struct Chain {
    /// Text size in pixels.
    size: f32,
    /// Caret targets as words came: when, where (right of the word, on its
    /// baseline) and on which line. In time order.
    targets: Vec<(f64, Point, usize)>,
    /// Where the caret waits before the first word.
    home: Point,
    /// Revised words: when and their box.
    plates: Vec<(f64, Rect)>,
    /// Onsets, in scene time.
    joints: Vec<f64>,
    /// Words an onset started: when they came and their box.
    ticks: Vec<(f64, Rect)>,
    /// The caret shows: listening.
    caret: bool,
    /// New words push the status plate.
    push: bool,
    reduced: bool,
    /// The box's size.
    box_size: Size,
}

/// Seconds a revised word's plate takes to slide out and back.
const PLATE_SECS: f64 = 0.14;
/// Seconds the onset joint takes to open and to close again.
const JOINT_OPEN: f64 = 0.035;
const JOINT_SECS: f64 = 0.11;
/// Seconds a baseline tick stays under its word.
const TICK_SECS: f64 = 0.12;

impl Chain {
    /// Arrival times of the words, for the status plate.
    fn pushes(&self) -> Vec<f64> {
        if !self.push {
            return Vec::new();
        }
        self.targets.iter().map(|&(at, _, _)| at).collect()
    }

    /// The caret's place at `t`: right of the newest word as soon as the
    /// word shows, so it never stands on the word's letters, pushed a little
    /// further on; one link later it settles back as a damped piston.
    /// Before the first word it waits at home.
    fn caret_at(&self, t: f64) -> Point {
        let Some(&(at, to, _)) = self.targets.iter().rev().find(|&&(at, _, _)| at <= t) else {
            return self.home;
        };
        if self.reduced {
            return to;
        }
        let kick = CARET_KICK * self.size * (1.0 - sounding::piston(t - at - LINK));
        Point::new(to.x + kick, to.y)
    }

    /// The scene keeps drawing until the last event has played out.
    fn until(&self) -> f64 {
        let last = self
            .targets
            .iter()
            .map(|e| e.0)
            .chain(self.plates.iter().map(|e| e.0))
            .chain(self.joints.iter().copied())
            .chain(self.ticks.iter().map(|e| e.0))
            .fold(f64::NEG_INFINITY, f64::max);
        last + 0.6
    }

    fn element(self) -> Element<Msg> {
        let until = self.until();
        let size = self.box_size;
        Element::canvas("chain", until, move |t, rect, p| self.draw(t, rect, p))
            .size(size.w, size.h)
    }

    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let s = self.size;
        let o = |r: Rect, dy: f32| Rect::new(rect.x + r.x, rect.y + r.y + dy, r.w, r.h);
        // Plates under revised words slide down out of their slot and back.
        for &(at, r) in &self.plates {
            let dt = t - at;
            if !(0.0..0.6).contains(&dt) {
                continue;
            }
            let (slide, a) = if self.reduced {
                (0.0, if dt < 0.3 { 0.8 } else { 0.0 })
            } else {
                let k = (dt / PLATE_SECS).clamp(0.0, 1.0);
                let slide = (std::f64::consts::PI * k).sin() as f32;
                (slide, 1.0 - ramp(dt, 0.25, 0.6))
            };
            let r = o(r, slide * 0.24 * s);
            let (x0, y0) = (r.x - 0.1 * s, r.y + 0.04 * s);
            let (x1, y1) = (r.x + r.w + 0.1 * s, r.y + r.h + 0.12 * s);
            let c = 0.12 * s;
            let pts = [
                (x0 + c, y0),
                (x1, y0),
                (x1, y1 - c),
                (x1 - c, y1),
                (x0, y1),
                (x0, y0 + c),
            ];
            p.fill(
                [&poly(&pts)],
                &StrokeStyle::solid(1.0, hex(FILL).fade(0.9 * a)),
            );
            p.stroke(
                &ring(&pts),
                &StrokeStyle::solid(1.4, hex(ORANGE).fade(0.85 * a)).glow(1.2),
            );
            // The rail it runs on.
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0 + c, y1 + 2.0, x1 - x0 - 2.0 * c, 1.5), 0.0),
                Paint::solid(hex(YELLOW).fade(0.7 * a * slide.max(0.3))),
            ));
        }
        // Baseline ticks: the joint the word's onset opened, under its
        // first letters.
        for &(at, r) in &self.ticks {
            let dt = t - at;
            if !(0.0..TICK_SECS).contains(&dt) {
                continue;
            }
            let a = 1.0 - ramp(dt, 0.0, TICK_SECS);
            let w = r.w.min(0.4 * s);
            let tr = Rect::new(rect.x + r.x, rect.y + r.y + r.h + 0.02 * s, w, 2.5);
            p.shape(ShapeInstance::rrect(
                RRect::new(tr, 0.0),
                Paint::glow(hex(YELLOW), 2.0, 0.6 * a),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(tr, 0.0),
                Paint::solid(hex(YELLOW).fade(a)),
            ));
        }
        if !self.caret {
            return;
        }
        // The caret: a block on a foot. An onset splits the foot at its
        // middle, 2 px apart, and it closes again.
        let c = self.caret_at(t);
        let (x, y) = (rect.x + c.x, rect.y + c.y);
        let block = Rect::new(x, y - 0.70 * s, 0.17 * s, 0.66 * s);
        p.shape(ShapeInstance::rrect(
            RRect::new(block, 0.0),
            Paint::glow(hex(DEEP), 3.0, 0.35),
        ));
        p.shape(ShapeInstance::rrect(
            RRect::new(block, 0.0),
            Paint::solid(hex(YELLOW)),
        ));
        let gap = self
            .joints
            .iter()
            .rev()
            .find(|&&at| at <= t)
            .map_or(0.0, |&at| {
                let dt = t - at;
                if self.reduced {
                    if dt < JOINT_SECS {
                        1.0
                    } else {
                        0.0
                    }
                } else if dt < JOINT_OPEN {
                    ease_out(ramp(dt, 0.0, JOINT_OPEN))
                } else {
                    1.0 - ramp(dt, JOINT_OPEN, JOINT_SECS)
                }
            })
            * 2.0;
        let foot_w = 0.42 * s;
        let fx = x + block.w / 2.0 - foot_w / 2.0;
        let fy = y + 0.06 * s;
        let half = (foot_w - gap) / 2.0;
        for (fx, w) in [(fx, half), (fx + half + gap, half)] {
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(fx, fy, w, 3.0), 0.0),
                Paint::solid(hex(ORANGE)),
            ));
        }
    }
}

impl Popup {
    /// The chain's events for the words on screen, from what was heard.
    fn mecha_chain(&self, phase: Phase) -> Chain {
        let l = &self.look;
        let s = scale(l);
        let style = text_style(l);
        let (boxes, lines) =
            sounding::word_boxes(&self.measure, &self.shown, &style, Some(text_width(l)));
        let box_size = Size::new(text_width(l), MAX_LINES as f32 * TEXT_LINE * s);
        // The text sits at the bottom of its box.
        let y0 = box_size.h - lines.max(1).min(MAX_LINES as usize) as f32 * style.line_height;
        // Clear of the word: the caret's foot reaches 0.125 em left of its
        // block, under the last letter's descender.
        let gap = 0.2 * style.size;
        let (hydraulic, joint) = (l.on(HYDRAULIC_CHAIN), l.on(ONSET_JOINT));
        let editing = self.edit_view.is_some();
        let mut chain = Chain {
            size: style.size,
            home: Point::new(0.0, y0 + style.line_height * 0.78),
            // While editing the edit caret is the one caret.
            caret: phase == Phase::Listening && (hydraulic || joint) && !editing,
            reduced: kit::reduced() || l.on("reduced-motion"),
            box_size,
            joints: if joint {
                self.heard.onsets().to_vec()
            } else {
                Vec::new()
            },
            ..Chain::default()
        };
        // An edit is no new word or revision: no push, plate or tick.
        if editing || (!hydraulic && !joint) {
            return chain;
        }
        if let Some(first) = boxes.first() {
            chain.home = Point::new(0.0, y0 + first.baseline);
        }
        let mut targets: Vec<(f64, Point, usize)> = Vec::new();
        for b in &boxes {
            let Some(h) = self.heard.word(b.id) else {
                continue;
            };
            let r = Rect::new(b.rect.x, y0 + b.rect.y, b.rect.w, b.rect.h);
            targets.push((
                h.born,
                Point::new(b.rect.x + b.rect.w + gap, y0 + b.baseline),
                b.line,
            ));
            match h.changed {
                Some(c) if hydraulic => chain.plates.push((c, r)),
                _ => {}
            }
            if joint && h.onset.is_some() {
                chain.ticks.push((h.born, r));
            }
        }
        targets.sort_by(|a, b| a.0.total_cmp(&b.0));
        chain.targets = targets;
        chain.push = hydraulic && !chain.reduced;
        chain
    }
}

impl Popup {
    /// R2-23: the words on screen in the text box's space, each with when
    /// it came and the voice it came at, oldest first.
    fn mecha_box_words(&self) -> Vec<(f64, Rect, f32)> {
        let l = &self.look;
        let s = scale(l);
        let style = text_style(l);
        let (boxes, lines) =
            sounding::word_boxes(&self.measure, &self.shown, &style, Some(text_width(l)));
        let box_h = MAX_LINES as f32 * TEXT_LINE * s;
        // The text sits at the bottom of its box.
        let y0 = box_h - lines.max(1).min(MAX_LINES as usize) as f32 * style.line_height;
        let levels = self.kit_levels();
        let mut words: Vec<(f64, Rect, f32)> = boxes
            .iter()
            .filter_map(|b| {
                let born = self.heard.word(b.id)?.born;
                let r = Rect::new(b.rect.x, y0 + b.rect.y, b.rect.w, b.rect.h);
                Some((born, r, levels.at(born)))
            })
            .collect();
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        words
    }

    /// R2-23-a, R2-23-b and R2-23-c behind the text: the glass slab, the
    /// contour grid and the ink particles, while listening.
    fn mecha_behind_text(&self, phase: Phase) -> Vec<Element<Msg>> {
        let l = &self.look;
        let (prism, grid, ink) = (l.on(GLASS_PRISM), l.on(BREATHING_GRID), l.on(MAGNETIC_INK));
        let mut out = Vec::new();
        if !(prism || grid || ink) || self.edit_view.is_some() || phase == Phase::Cancelled {
            return out;
        }
        let s = scale(l);
        let still = kit::reduced() || l.on("reduced-motion");
        if prism {
            // The slab lies just inside the panel's border, its corners cut
            // as the panel's are.
            let inset = 4.0;
            let corners = [24.0, 24.0, 43.0, 78.0].map(|c| Corner::Chamfer((c - inset * 0.6) * s));
            let glass = Glass {
                tint: hex(PANEL).fade(0.35),
                accent: hex(ORANGE),
                rim: 0.35,
                bevel: 10.0 * s,
                shadow: [0.0; 3],
                blur_behind: false,
                ..Glass::dark()
            }
            .with_shape(GlassShape::Corners(corners));
            let glass = if still { glass.still() } else { glass };
            let slab = Element::panel("glass-slab")
                .size((862.0 - 2.0 * inset) * s, (234.0 - 2.0 * inset) * s)
                .glass(glass)
                .presence(Transition::fade(), Transition::fade());
            out.push(at(slab, (72.0 + inset) * s, (300.0 + inset) * s));
        }
        if phase != Phase::Listening {
            return out;
        }
        let words = self.mecha_box_words();
        let newest = words.last().copied();
        // The canvases cover the panel; their origin is the text box's.
        let (ox, oy) = (TEXT_X * s, TEXT_Y * s);
        let panel = Size::new(980.0 * s, 630.0 * s);
        if grid {
            let levels = self.kit_levels();
            let until = if still { 0.0 } else { f64::INFINITY };
            let canvas = Element::canvas("breathing-grid", until, move |t, r, p| {
                let rms = if still {
                    0.0
                } else {
                    let v = levels.at(t);
                    if v < VOICE_GATE {
                        0.0
                    } else {
                        (v * 2.2).min(1.0)
                    }
                };
                let mut y = 300.0 + GRID_PITCH;
                let mut i = 0;
                while y < 534.0 - GRID_PITCH * 0.5 {
                    let (x0, x1) = panel_span(y, 14.0);
                    let pts: Vec<Point> = (0..=60)
                        .map(|j| {
                            let x = x0 + (x1 - x0) * j as f32 / 60.0;
                            // In the text box's space, pixels.
                            let (bx, by) = ((x - TEXT_X) * s, (y - TEXT_Y) * s);
                            let mut lift = 0.0;
                            if let (Some((born, w, _)), false) = (newest, still) {
                                let k = grid_lift(t - born);
                                if k > 0.0 {
                                    let dx = (w.x - bx).max(bx - (w.x + w.w)).max(0.0);
                                    let dy = (w.y - by).max(by - (w.y + w.h)).max(0.0);
                                    let d = (dx * dx + dy * dy).sqrt() / (GRID_REACH * s);
                                    lift = GRID_LIFT * k * (-d * d).exp();
                                }
                            }
                            let wave = GRID_WAVE
                                * rms
                                * (0.035 * x - 3.0 * t as f32 + 0.9 * i as f32).sin();
                            Point::new(r.x + ox + bx, r.y + oy + by - lift + wave)
                        })
                        .collect();
                    p.stroke(
                        &Contour::polyline(&pts, false),
                        &StrokeStyle::solid(1.0, hex(ORANGE).fade(GRID_ALPHA)),
                    );
                    y += GRID_PITCH;
                    i += 1;
                }
            })
            .size(panel.w, panel.h);
            out.push(at(canvas, 0.0, 0.0));
        }
        if ink && !still {
            let last = words.last().map_or(0.0, |w| w.0);
            let canvas =
                Element::canvas("magnetic-ink", last + INK_IN + INK_OUT, move |t, r, p| {
                    for (wi, &(born, w, level)) in words.iter().enumerate().rev() {
                        let age = t - born;
                        if age >= INK_IN + INK_OUT {
                            break;
                        }
                        if age < 0.0 {
                            continue;
                        }
                        let n =
                            INK_DOTS.0 + (INK_DOTS.1 - INK_DOTS.0) * (level * 2.2).clamp(0.0, 1.0);
                        let seed = (born * 1000.0) as i64 * 131 + wi as i64;
                        for i in 0..n as i64 {
                            let (a, b) = (noise(seed * 977 + i), noise(seed * 977 + i + 4001));
                            let to = Point::new(w.x + a * w.w, w.y + 0.15 * w.h + b * 0.7 * w.h);
                            // From the panel's left edge, near the word's line.
                            let from = Point::new((82.0 - TEXT_X) * s, to.y + (b - 0.5) * 60.0 * s);
                            let Some((d, alpha)) = ink_dot(age, from, to, seed * 977 + i, s) else {
                                continue;
                            };
                            let rad = (0.9 + 0.8 * a) * s.max(0.6);
                            let c = mix(hex(ORANGE), hex(YELLOW), b);
                            let rr = RRect::new(
                                Rect::new(
                                    r.x + ox + d.x - rad,
                                    r.y + oy + d.y - rad,
                                    2.0 * rad,
                                    2.0 * rad,
                                ),
                                rad,
                            );
                            p.shape(ShapeInstance::rrect(rr, Paint::solid(c.fade(alpha))));
                        }
                    }
                })
                .size(panel.w, panel.h);
            out.push(at(canvas, 0.0, 0.0));
        }
        out
    }

    /// R2-23-b: a spectral band across the glass slab for each new word,
    /// over the text, additive, while listening.
    fn mecha_sweep(&self, phase: Phase) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(GLASS_PRISM)
            || phase != Phase::Listening
            || self.edit_view.is_some()
            || kit::reduced()
            || l.on("reduced-motion")
        {
            return None;
        }
        let s = scale(l);
        let borns: Vec<f64> = self.mecha_box_words().iter().map(|w| w.0).collect();
        let last = *borns.last()?;
        let canvas = Element::canvas("prism-sweep", last + SWEEP_SECS, move |t, r, p| {
            // Only the newest word's band, so two never add up.
            let Some((u, alpha)) = borns.iter().rev().find_map(|&born| sweep(t - born)) else {
                return;
            };
            // The band's middle runs from the panel's left edge to its
            // right: the colours blend into each other and fade out at both
            // sides.
            let (x0, x1) = (72.0 - SWEEP_W / 2.0, 934.0 + SWEEP_W / 2.0);
            let left = x0 + (x1 - x0) * u - SWEEP_W / 2.0;
            let n = SWEEP_COLOURS.len();
            let stops: Vec<Color> = std::iter::once(hex(SWEEP_COLOURS[0]).fade(0.0))
                .chain(SWEEP_COLOURS.iter().map(|&c| hex(c).fade(alpha)))
                .chain(std::iter::once(hex(SWEEP_COLOURS[n - 1]).fade(0.0)))
                .collect();
            let step = SWEEP_W / (stops.len() - 1) as f32;
            for (i, pair) in stops.windows(2).enumerate() {
                let a = left + step * i as f32;
                let b = a + step;
                // Clipped to the slab.
                let quad = [(a, 300.0), (b, 300.0), (b, 534.0), (a, 534.0)];
                let pts: Vec<Point> = clip_convex(&quad, &SLAB)
                    .into_iter()
                    .map(|(x, y)| Point::new(r.x + x * s, r.y + y * s))
                    .collect();
                if pts.len() < 3 {
                    continue;
                }
                let mid = r.y + 417.0 * s;
                p.fill(
                    [&Contour::polyline(&pts, true)],
                    &StrokeStyle::solid(1.0, pair[0]).linear(
                        Point::new(r.x + a * s, mid),
                        pair[0],
                        Point::new(r.x + b * s, mid),
                        pair[1],
                    ),
                );
            }
        })
        .size(980.0 * s, 630.0 * s)
        .blend(BlendMode::Add);
        Some(at(canvas, 0.0, 0.0))
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use super::Phase;
    use voxtype::osd::config::OsdTheme;

    /// M7.L5: Mecha edits in its own layout, its slide-in entry off (the
    /// still profile). With the caret on the newest, a middle and the top
    /// line, at both widths: the caret on its glyph within 1 px in every
    /// frame, the popup's size unchanged, and the golden
    /// `tests/fixtures/osd-vo/edit-mecha.txt`.
    #[test]
    fn mecha_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Mecha, "mecha");
    }

    /// M7.L5: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn mecha_edits_within_the_envelope() {
        envelope(OsdTheme::Mecha, "mecha");
    }

    /// Labels shaped with one font table are not reused for another, where
    /// the same font id is another face.
    #[test]
    fn shaped_labels_are_kept_per_font_table() {
        use super::super::*;
        let plasma = PlasmaTheme::default();
        let (a, b) = (load_fonts(&plasma).unwrap(), load_fonts(&plasma).unwrap());
        let id = a.add_system("monospace").unwrap();
        assert_eq!(b.add_system("serif").unwrap(), id);
        let (ma, mb) = (vo::ShapedLayout::new(a), vo::ShapedLayout::new(b));
        let text = "MMMM iiii";
        let wa = super::shaped(&ma, text, id, 20.0).size.w;
        let wb = super::shaped(&mb, text, id, 20.0).size.w;
        assert_ne!(wa, wb, "a monospace and a serif face");
    }

    /// The hydraulic chain's caret block shows while listening; while
    /// editing the edit caret is the one caret, and an edit pushes no
    /// plate.
    #[test]
    fn mecha_hides_its_chain_caret_while_editing() {
        let mut r = Rig::new(OsdTheme::Mecha, false);
        r.dictate(40);
        assert!(r.p.mecha_chain(Phase::Listening).caret, "listening");
        r.enter(1.0);
        let chain = r.p.mecha_chain(Phase::Listening);
        assert!(!chain.caret && chain.plates.is_empty() && chain.targets.is_empty());
    }
}

#[cfg(test)]
mod round2_tests {
    use super::*;

    #[test]
    fn the_particles_are_gone_500_ms_after_they_gather() {
        let (from, to) = (Point::new(0.0, 50.0), Point::new(200.0, 40.0));
        for seed in 0..40 {
            assert!(ink_dot(-0.01, from, to, seed, 1.0).is_none());
            assert!(ink_dot(INK_IN + INK_OUT, from, to, seed, 1.0).is_none());
            for i in 0..80 {
                let age = i as f64 * 0.01;
                if let Some((_, a)) = ink_dot(age, from, to, seed, 1.0) {
                    assert!((0.0..=INK_ALPHA).contains(&a), "{a}");
                }
            }
            // Gathered at the word once they have flowed in.
            let (p, _) = ink_dot(INK_IN, from, to, seed, 1.0).unwrap();
            assert!((p.x - to.x).abs() < 1e-3 && (p.y - to.y).abs() < 1e-3);
        }
        assert_eq!(
            ink_dot(0.42, from, to, 9, 1.0),
            ink_dot(0.42, from, to, 9, 1.0)
        );
    }

    #[test]
    fn the_sweep_stays_within_15_percent_and_crosses_once() {
        assert!(sweep(-0.01).is_none());
        assert!(sweep(SWEEP_SECS).is_none());
        let mut last = -1.0;
        for i in 0..60 {
            let (u, a) = sweep(i as f64 * 0.01).unwrap();
            assert!((0.0..=0.15).contains(&a));
            assert!(u >= last);
            last = u;
        }
    }

    #[test]
    fn the_lift_is_gone_after_800_ms() {
        assert_eq!(grid_lift(-0.1), 0.0);
        assert!(grid_lift(0.12) > 0.99);
        assert_eq!(grid_lift(GRID_SECS), 0.0);
        assert_eq!(grid_lift(GRID_SECS + 1.0), 0.0);
        for i in 0..90 {
            let k = grid_lift(i as f64 * 0.01);
            assert!((0.0..=1.0).contains(&k));
            assert!(GRID_LIFT * k <= 4.0);
        }
    }

    #[test]
    fn a_band_is_clipped_to_the_slab() {
        // Inside: unchanged. Outside: nothing. Across the left chamfer: cut.
        let inside = [
            (300.0, 350.0),
            (320.0, 350.0),
            (320.0, 400.0),
            (300.0, 400.0),
        ];
        assert_eq!(clip_convex(&inside, &SLAB).len(), 4);
        let outside = [(0.0, 350.0), (20.0, 350.0), (20.0, 400.0), (0.0, 400.0)];
        assert!(clip_convex(&outside, &SLAB).is_empty());
        let cut = clip_convex(
            &[(60.0, 300.0), (130.0, 300.0), (130.0, 534.0), (60.0, 534.0)],
            &SLAB,
        );
        assert!(cut
            .iter()
            .all(|&(x, y)| x >= 76.0 - 1e-3 && (300.0..=534.0).contains(&y)));
        assert!(cut.len() >= 4);
    }

    #[test]
    fn the_panel_span_follows_the_border() {
        assert_eq!(panel_span(300.0, 0.0), (96.0, 910.0));
        assert_eq!(panel_span(400.0, 0.0), (72.0, 934.0));
        assert_eq!(panel_span(534.0, 0.0), (150.0, 892.0));
    }
}
