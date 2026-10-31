//! The `island` look of the vo popup, after mock-up 03: a black capsule in
//! the style of a phone's dynamic island. It opens as a dot, springs out to
//! a pill with a red recording dot, a clock, a small blue to violet bar
//! waveform, up to three lines of words and a microphone button, and grows
//! into a card with a header row once the words need a fourth line. The
//! microphone turns into a green check when the text is pasted; a cancelled
//! dictation shrinks back to a dot.
//!
//! The capsule, the waveform, the dots and the icons are drawn on one canvas
//! from the frame time, so the size springs and the dot pulses without new
//! views. The words and the clock are live text and a digit roll. Short
//! ticks run only while the capsule changes shape.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Colours sampled from the mock-up.
const BODY: u32 = 0x030407;
const RIM: u32 = 0x34405a;
const HALO: u32 = 0x2f6bff;
const RED: u32 = 0xff3b30;
const WHITE: u32 = 0xeef1f6;
const DIM: u32 = 0x8a91a0;
const STATE: u32 = 0x9fb6dc;
const HINT: u32 = 0x7f8aa3;
const BLUE: u32 = 0x2d8cff;
const CYAN: u32 = 0x22b8ff;
const VIOLET: u32 = 0x8b5cf6;
const BUTTON: u32 = 0x15181f;
const ICON: u32 = 0xc4cad6;
const GREEN: u32 = 0x34d27b;
const GREEN_BG: u32 = 0x0f3826;
const TRACK: u32 = 0x262b36;

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: i64) -> f32 {
    let mut x = (n as u64 as u32).wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
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

/// The capsule's width: the mock-up's 576 px at 1.04, whatever the
/// configured popup width, since the layout is drawn for it.
const WIDTH: f32 = 600.0;
/// Room round the capsule for its glow.
const MARGIN_X: f32 = 24.0;
const MARGIN_Y: f32 = 18.0;

/// The pill: text column, line height and padding, after the mock-up at
/// 600 px wide.
const PILL_TEXT_X: f32 = 244.0;
const PILL_TEXT_PX: f32 = 14.0;
const PILL_LINE: f32 = 19.2;
const PILL_PAD_TOP: f32 = 15.0;
const PILL_PAD_BOTTOM: f32 = 13.0;
const PILL_LINES: u32 = 3;
/// Room the microphone button takes at the right end of the pill.
const PILL_BUTTON: f32 = 74.0;
const STATUS_PX: f32 = 11.6;
const STATUS_H: f32 = 22.0;
/// The progress bar row while finishing.
const BAR_ROW: f32 = 20.0;

/// The card: header, text and status rows.
const CARD_HEADER: f32 = 86.0;
const CARD_PAD_X: f32 = 30.0;
const CARD_TEXT_PX: f32 = 15.6;
const CARD_LINE: f32 = 25.0;
const CARD_LINES: u32 = 6;
const CARD_STATUS: f32 = 50.0;
const CARD_RADIUS: f32 = 34.0;

/// The dot the capsule opens from and shrinks back to.
const DOT_SIZE: f32 = 40.0;
/// The small pill while the model loads, at least this wide.
const LOAD_W: f32 = 250.0;
/// Room after the loading label for the forming cue (about 2 letters),
/// and for the percentage with the pill's right end.
const LOAD_CUE: f32 = 18.0;
const LOAD_PCT: f32 = 58.0;
const LOAD_STEP: f32 = 16.0;
const LOAD_H: f32 = 46.0;

/// Waveform bars.
const PILL_BARS: usize = 30;
const CARD_BARS: usize = 44;
/// Seconds of audio between neighbouring bars.
const BAR_SECS: f64 = 0.035;

/// Morph timing: the capsule's spring settles in about this long, and the
/// content fades in after this delay.
const MORPH_SECS: f64 = 0.42;
const CONTENT_DELAY: f64 = 0.12;
const CONTENT_FADE: f64 = 0.18;
/// A status label goes out in `SWAP_OUT`, then the next one comes in over
/// `SWAP_IN`. Text that leaves with its form is gone within `SWAP_OUT` too,
/// before the new form's content starts at `CONTENT_DELAY`.
const SWAP_OUT: f64 = 0.08;
const SWAP_IN: f64 = 0.14;
/// Pixels the capsule's bottom edge may still be short of its place when
/// the words of a new line show.
const ROOM_GAP: f32 = 3.0;
const TICK_HZ: f64 = 60.0;
/// Word bulge (MO-74): each new word pushes the capsule's right edge out
/// this far, quickly, and it relaxes.
const BULGE_PX: f32 = 4.0;
const BULGE_RISE: f64 = 0.05;
const BULGE_SECS: f64 = 0.2;
/// Idle motion while listening: a soft light orbits the capsule's edge,
/// and the flat bars rise by at most 1 px with the room's noise floor.
const ORBIT_SECS: f64 = 6.0;
const ORBIT_GLOW: f32 = 12.0;
const ORBIT_ALPHA: f32 = 0.35;
const FLOOR_LIFT: f32 = 1.0;
/// The level that lifts a flat bar the whole `FLOOR_LIFT`; from four times
/// it the lift is gone, so speech draws as before.
const FLOOR_REF: f32 = 0.004;

/// The point a fraction `u` of the way round `r`'s edge, clockwise from
/// the start of the top side.
fn edge_point(r: RRect, u: f32) -> Point {
    let b = r.rect;
    let rad = r.radii[0].min(b.w / 2.0).min(b.h / 2.0);
    let (sw, sh) = (b.w - 2.0 * rad, b.h - 2.0 * rad);
    let arc = std::f32::consts::FRAC_PI_2 * rad;
    let mut d = u.rem_euclid(1.0) * 2.0 * (sw + sh + 2.0 * arc);
    // Each side, then the corner after it: its centre and start angle.
    let sides = [
        (sw, Point::new(b.x + rad, b.y), Point::new(1.0, 0.0)),
        (sh, Point::new(b.x + b.w, b.y + rad), Point::new(0.0, 1.0)),
        (
            sw,
            Point::new(b.x + b.w - rad, b.y + b.h),
            Point::new(-1.0, 0.0),
        ),
        (sh, Point::new(b.x, b.y + b.h - rad), Point::new(0.0, -1.0)),
    ];
    let corners = [
        (
            Point::new(b.x + b.w - rad, b.y + rad),
            -std::f32::consts::FRAC_PI_2,
        ),
        (Point::new(b.x + b.w - rad, b.y + b.h - rad), 0.0),
        (
            Point::new(b.x + rad, b.y + b.h - rad),
            std::f32::consts::FRAC_PI_2,
        ),
        (Point::new(b.x + rad, b.y + rad), std::f32::consts::PI),
    ];
    for ((len, start, dir), (c, a0)) in sides.into_iter().zip(corners) {
        if d <= len {
            return Point::new(start.x + dir.x * d, start.y + dir.y * d);
        }
        d -= len;
        if d <= arc {
            let a = a0 + d / rad.max(1e-3);
            return Point::new(c.x + rad * a.cos(), c.y + rad * a.sin());
        }
        d -= arc;
    }
    Point::new(b.x + rad, b.y)
}

/// How far the right edge is out `s` seconds after a word came.
fn bulge(s: f64) -> f32 {
    if !(0.0..BULGE_SECS).contains(&s) {
        0.0
    } else if s < BULGE_RISE {
        BULGE_PX * ease_out(s / BULGE_RISE)
    } else {
        let k = (s - BULGE_RISE) / (BULGE_SECS - BULGE_RISE);
        BULGE_PX * (1.0 - ease_out(k))
    }
}

#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    medium: u32,
}

/// The look's fonts, added to `reg` once (`FontRegistry::system`), so
/// every registry, such as a test's, has its own ids.
fn fonts(reg: &FontRegistry) -> Fonts {
    let add = |pattern: &str, or: u32| reg.system(pattern).unwrap_or(or);
    Fonts {
        text: add("Nunito:semibold", SANS),
        medium: add("Nunito:medium", SANS),
    }
}

fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

fn card_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, CARD_TEXT_PX, CARD_LINE).tracking(0.1)
}

fn pill_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, PILL_TEXT_PX, PILL_LINE).tracking(0.05)
}

/// The popup counts lines in the card, the largest the capsule gets.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    card_style(look)
}

pub(crate) fn text_width(_look: &Look) -> f32 {
    WIDTH - 2.0 * CARD_PAD_X
}

pub(crate) const MAX_LINES: u32 = CARD_LINES;

fn pill_text_w(w: f32) -> f32 {
    w - PILL_TEXT_X - PILL_BUTTON
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

/// The step response of a spring that overshoots a little and settles in
/// about `MORPH_SECS`.
fn spring(s: f64) -> f32 {
    if reduced() {
        return 1.0;
    }
    if s <= 0.0 {
        return 0.0;
    }
    let zeta = 0.68f64;
    let omega = 4.6 / (zeta * MORPH_SECS);
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    let e = (-zeta * omega * s).exp();
    (1.0 - e * ((wd * s).cos() + zeta * omega / wd * (wd * s).sin())) as f32
}

/// What the popup shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Loading,
    Listening,
    Finishing,
    Done,
    Cancelled,
}

impl From<kit::Mode> for Mode {
    fn from(mode: kit::Mode) -> Self {
        match mode {
            kit::Mode::Loading => Mode::Loading,
            kit::Mode::Listening => Mode::Listening,
            kit::Mode::Finishing => Mode::Finishing,
            kit::Mode::Done => Mode::Done,
            kit::Mode::Cancelled => Mode::Cancelled,
        }
    }
}

/// The capsule's size and corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    w: f32,
    h: f32,
    r: f32,
}

impl Geo {
    fn lerp(a: Geo, b: Geo, k: f32) -> Geo {
        Geo {
            w: a.w + (b.w - a.w) * k,
            h: a.h + (b.h - a.h) * k,
            r: a.r + (b.r - a.r) * k,
        }
    }
}

/// A capsule shape change: from, to and when.
#[derive(Clone, Copy, Debug)]
struct Shape {
    from: Geo,
    to: Geo,
    at: f64,
}

impl Shape {
    fn at(&self, t: f64) -> Geo {
        let k = spring(t - self.at);
        let mut g = Geo::lerp(self.from, self.to, k);
        // An overshoot must never turn the corners inside out.
        g.w = g.w.max(1.0);
        g.h = g.h.max(1.0);
        g.r = g.r.clamp(0.0, 0.5 * g.w.min(g.h));
        g
    }
}

/// State only the island look keeps.
pub struct State {
    mode: Option<Mode>,
    mode_at: f64,
    /// The capsule grew into the card; it stays a card.
    card: bool,
    shape: Shape,
    /// The progress bar's eased fraction: from, to, when.
    prog: (f32, f32, f64),
    /// When the newest word came, for the caret.
    word_at: f64,
    words: usize,
    /// Lines of words the capsule holds now, and held before its latest
    /// change of shape.
    lines: u32,
    room: u32,
    /// The form of the latest shape (pill or card, or the loading pill or
    /// the cancel dot), and when its content last faded in. A new line or
    /// a new step of the dictation only resizes the capsule and the content
    /// rides on its edge; a new form fades the content in.
    kind: Option<(bool, Option<Mode>)>,
    fade_at: f64,
    /// The status row, and the one before it with when the label changed:
    /// the old row goes out before the new one comes in.
    status: Option<StatusText>,
    old: Option<StatusText>,
    label_at: f64,
}

/// What the status row says, and how it is set.
#[derive(Clone)]
struct StatusText {
    state: String,
    detail: String,
    colour: Color,
    finishing: bool,
    done: bool,
}

impl Default for State {
    fn default() -> State {
        let dot = Geo {
            w: DOT_SIZE,
            h: DOT_SIZE,
            r: DOT_SIZE / 2.0,
        };
        State {
            mode: None,
            mode_at: f64::NEG_INFINITY,
            card: false,
            shape: Shape {
                from: dot,
                to: dot,
                at: 0.0,
            },
            prog: (0.0, 0.0, 0.0),
            word_at: f64::NEG_INFINITY,
            words: 0,
            lines: 0,
            room: 0,
            kind: None,
            fade_at: 0.0,
            status: None,
            old: None,
            label_at: f64::NEG_INFINITY,
        }
    }
}

/// Everything the canvas draws, copied out of the popup for one view.
#[derive(Clone)]
struct Scene {
    surface: Size,
    shape: Shape,
    mode: Mode,
    mode_at: f64,
    card: bool,
    shown_at: f64,
    /// When the content of the current shape fades in.
    content_at: f64,
    levels: Arc<[f32]>,
    /// How far the chrome has exhaled (MO-12), 0 awake to 1 at rest.
    rest: f32,
    newest_at: f64,
    finish_at: Option<f64>,
    /// Progress, eased: from, to, when; `None` hides the bar.
    prog: Option<(f32, f32, f64)>,
    /// The bar's place, relative to the capsule's left and top.
    bar: Rect,
    /// The status dot's centre, relative to the capsule.
    status_dot: Point,
    status_colour: Color,
    /// The dot comes in with a new status label.
    status_alpha: f32,
    /// The caret after the newest word, relative to the capsule, and when
    /// the newest word came.
    caret: Option<(Point, f64)>,
    /// Card rules, relative to the capsule's top, and whether each rides
    /// on the capsule's bottom edge as it grows.
    rules: Vec<(f32, bool)>,
    /// When the recent words came, for the bulge.
    words_at: Vec<f64>,
    /// Reduced motion: no idle orbit or floor lift.
    still: bool,
}

impl Scene {
    fn origin(&self, g: Geo) -> Point {
        Point::new((self.surface.w - g.w) / 2.0, MARGIN_Y)
    }

    fn content(&self, t: f64) -> f32 {
        if reduced() {
            return 1.0;
        }
        ease_out((t - self.content_at) / CONTENT_FADE)
    }

    /// How far the right edge bulges at `t`.
    fn bulge(&self, t: f64) -> f32 {
        if self.mode != Mode::Listening {
            return 0.0;
        }
        self.words_at
            .iter()
            .map(|&at| bulge(t - at))
            .fold(0.0, f32::max)
    }

    fn until(&self, now: f64) -> f64 {
        let words = self
            .words_at
            .iter()
            .fold(0.0, |a: f64, &b| a.max(b + BULGE_SECS));
        let morph = (self.shape.at + MORPH_SECS + 0.2).max(words);
        match self.mode {
            // The dot pulses and the bars scroll.
            Mode::Loading | Mode::Listening => (now + 0.25).max(morph),
            Mode::Finishing => (now + 0.5).max(morph),
            _ => (now + 0.2).max(morph).max(self.mode_at + 0.6),
        }
    }

    fn idle(&self) -> bool {
        self.mode == Mode::Listening && !self.still
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
        let g = self.shape.at(t);
        let o = self.origin(g);
        // A new word pushes the right edge out a little; the left edge and
        // the content stay put.
        let body = RRect::new(Rect::new(o.x, o.y, g.w + self.bulge(t), g.h), g.r);
        if self.idle() {
            // Under everything, with an opaque underlay over it, so only its
            // outer half shows past the edge: the body is 2 % see-through,
            // and the light showed through it under the newest line.
            let u = ((t - self.shown_at) / ORBIT_SECS).fract() as f32;
            p.shape(ShapeInstance::circle(
                edge_point(body, u),
                ORBIT_GLOW / 2.0,
                Paint::glow(hex(HALO).fade(ORBIT_ALPHA), ORBIT_GLOW / 2.0, 0.5),
            ));
            p.shape(ShapeInstance::rrect(body, Paint::solid(hex(BODY))));
        }
        // The blue light round the edge and a soft shadow under it.
        p.shape(ShapeInstance::rrect(
            body,
            Paint::shadow(Color::BLACK.fade(0.55), Point::new(0.0, 4.0), 10.0, 0.0),
        ));
        p.shape(ShapeInstance::rrect(
            body,
            Paint::glow(hex(HALO).fade(0.30 * (1.0 - 0.6 * self.rest)), 7.0, 0.55),
        ));
        p.shape(
            ShapeInstance::rrect(body, Paint::solid(hex(BODY).fade(0.98))).with_stroke(
                Paint::linear(
                    Point::new(o.x, o.y),
                    hex(RIM),
                    Point::new(o.x, o.y + g.h),
                    hex(RIM).fade(0.25),
                ),
                1.0,
            ),
        );
        let k = self.content(t);
        if k <= 0.0 {
            return;
        }
        p.layer(Affine::translate(o.x, o.y), k, |p| {
            self.draw_content(t, g, p)
        });
    }

    fn draw_content(&self, t: f64, g: Geo, p: &mut Painter) {
        // What sits under the words rides on the bottom edge: while the
        // capsule grows for a new line, it is `drop` above its place.
        let drop = g.h - self.shape.to.h;
        for &(y, rides) in &self.rules {
            let y = if rides { y + drop } else { y };
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(CARD_PAD_X, y, g.w - 2.0 * CARD_PAD_X, 1.0), 0.0),
                Paint::solid(Color::WHITE.fade(0.08)),
            ));
        }
        let (head_y, wave) = if self.card {
            let cy = CARD_HEADER / 2.0;
            let half = (CARD_BARS as f32 - 1.0) * 4.6 / 2.0;
            (cy, (g.w / 2.0 - half, g.w / 2.0 + half, 26.0, CARD_BARS))
        } else {
            (g.h / 2.0, (96.0, 218.0, 20.0, PILL_BARS))
        };
        match self.mode {
            Mode::Loading => self.draw_loading(t, g, p),
            Mode::Finishing => {
                self.draw_stop(Point::new(32.0, head_y), p);
                self.draw_wave(t, wave, head_y, p);
            }
            _ => {
                self.draw_rec_dot(t, Point::new(28.0, head_y), p);
                self.draw_wave(t, wave, head_y, p);
            }
        }
        if self.mode != Mode::Loading {
            let r = if self.card { 27.0 } else { 23.0 };
            let c = Point::new(g.w - r - 16.0, head_y);
            self.draw_button(t, c, r, p);
            self.draw_status_dot(drop, p);
        }
        if let Some((a, b, at)) = self.prog {
            let f = a + (b - a) * ease_out((t - at) / 0.35);
            let mut bar = self.bar;
            bar.y += drop;
            let rr = bar.h / 2.0;
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, rr),
                Paint::solid(hex(TRACK)),
            ));
            if f > 0.0 {
                let fill = Rect::new(bar.x, bar.y, (bar.w * f).max(bar.h), bar.h);
                p.shape(ShapeInstance::rrect(
                    RRect::new(fill, rr),
                    Paint::glow(hex(BLUE).fade(0.6), 4.0, 0.5),
                ));
                p.shape(ShapeInstance::rrect(
                    RRect::new(fill, rr),
                    Paint::linear(
                        Point::new(bar.x, 0.0),
                        hex(BLUE),
                        Point::new(bar.x + bar.w, 0.0),
                        hex(CYAN),
                    ),
                ));
            }
        }
        if let Some((c, since)) = self.caret {
            // Solid while words come, then a slow blink.
            let idle = t - since - 0.4;
            let on = if idle < 0.0 || reduced() {
                1.0
            } else {
                0.5 + 0.5 * ((idle * std::f64::consts::TAU * 0.9).cos() as f32)
            };
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(c.x, c.y, 2.0, CARD_LINE * 0.72), 1.0),
                Paint::solid(hex(BLUE).fade(on)),
            ));
        }
    }

    /// The red recording dot: a slow pulse, with a soft red light.
    fn draw_rec_dot(&self, t: f64, c: Point, p: &mut Painter) {
        let pulse = if reduced() || self.mode != Mode::Listening {
            0.0
        } else {
            (0.5 - 0.5 * ((t - self.shown_at) * std::f64::consts::TAU).cos()) as f32
        };
        let r = 6.5 * (1.0 + 0.12 * pulse);
        let colour = if self.mode == Mode::Cancelled {
            hex(DIM)
        } else {
            hex(RED)
        };
        p.shape(ShapeInstance::circle(
            c,
            r,
            Paint::glow(colour.fade(0.3 + 0.2 * pulse), 4.0, 0.45),
        ));
        p.shape(ShapeInstance::circle(c, r, Paint::solid(colour)));
    }

    /// The stop square in its dark circle, after the recording stops.
    fn draw_stop(&self, c: Point, p: &mut Painter) {
        p.shape(ShapeInstance::circle(c, 20.0, Paint::solid(hex(BUTTON))));
        let s = 13.0;
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(c.x - s / 2.0, c.y - s / 2.0, s, s), 2.0),
            Paint::solid(hex(WHITE)),
        ));
    }

    fn draw_loading(&self, t: f64, g: Geo, p: &mut Painter) {
        let c = Point::new(23.0, g.h / 2.0);
        p.stroke(&make::ring(c, 8.0), &StrokeStyle::solid(2.0, hex(TRACK)));
        p.stroke(
            &make::spinner(c, 8.0, t - self.shown_at),
            &StrokeStyle::solid(2.0, hex(BLUE)),
        );
    }

    /// Bars that mirror out from the middle, the newest audio in the
    /// middle, blue on the left to violet on the right with a soft light.
    fn draw_wave(&self, t: f64, wave: (f32, f32, f32, usize), cy: f32, p: &mut Painter) {
        let (x0, x1, max_half, n) = wave;
        let pitch = (x1 - x0) / (n - 1) as f32;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let settle = self
            .finish_at
            .map_or(1.0, |f| 1.0 - 0.45 * ease_out((t - f) / 0.5));
        let mid = (n as f32 - 1.0) * 0.42;
        let grey = self.mode == Mode::Cancelled;
        for i in 0..n {
            let u = i as f32 / (n - 1) as f32;
            let d = (i as f32 - mid).abs();
            // The right side tapers faster, as in the mock-up.
            let span = if (i as f32) < mid {
                mid
            } else {
                n as f32 - 1.0 - mid
            };
            let reach = (d / span.max(1.0)).min(1.0);
            let s = t_eff - d as f64 * BAR_SECS;
            let level = self.level_at(s);
            // Neighbouring bars differ, as in the mock-up; each bar keeps its
            // own factor as it scrolls.
            let k = (s / BAR_SECS).floor() as i64;
            let jitter = 0.55 + 0.45 * noise(k);
            let taper = (1.0 - reach.powf(1.6)).max(0.0);
            let norm = ((level * 2.6).min(1.0) * jitter * (0.3 + 0.7 * taper)).max(0.0) * settle;
            let mut floor = 2.2;
            if self.idle() {
                // Room tone lifts a flat bar a little; speech fades it out.
                let fade = 1.0 - ((level - FLOOR_REF) / (3.0 * FLOOR_REF)).clamp(0.0, 1.0);
                floor += FLOOR_LIFT * (level / FLOOR_REF).min(1.0) * jitter * fade;
            }
            let half = (norm * max_half).max(floor);
            let x = x0 + i as f32 * pitch;
            let mut c = if u < 0.5 {
                mix(hex(CYAN), hex(BLUE), u * 2.0)
            } else {
                mix(hex(BLUE), hex(VIOLET), (u - 0.5) * 2.0)
            };
            if grey {
                c = mix(c, hex(DIM), 0.8).fade(0.6);
            }
            let r = Rect::new(x - 1.5, cy - half, 3.0, 2.0 * half);
            p.shape(ShapeInstance::capsule(
                r,
                Paint::glow(c.fade(0.4), 3.0, 0.3),
            ));
            p.shape(ShapeInstance::capsule(r, Paint::solid(c)));
        }
    }

    /// The microphone button, which turns into a green check once the text
    /// is pasted.
    fn draw_button(&self, t: f64, c: Point, r: f32, p: &mut Painter) {
        let done = self.mode == Mode::Done;
        let k = if done {
            if reduced() {
                1.0
            } else {
                spring(t - self.mode_at)
            }
        } else {
            0.0
        };
        let bg = mix(hex(BUTTON), hex(GREEN_BG), k.min(1.0));
        if k > 0.0 {
            p.shape(ShapeInstance::circle(
                c,
                r,
                Paint::glow(hex(GREEN).fade(0.35 * k.min(1.0)), 6.0, 0.6),
            ));
        }
        p.shape(ShapeInstance::circle(c, r, Paint::solid(bg)));
        let icon =
            |path: kurbo_path::BezPath, scale: f32, alpha: f32, colour: Color, p: &mut Painter| {
                let o = Outline::new(&path, 0.05).transform(
                    &Affine::translate(-12.0, -12.0)
                        .then(&Affine::scale(scale))
                        .then(&Affine::translate(c.x, c.y)),
                );
                p.outline(&o, &StrokeStyle::solid(1.7 * scale, colour.fade(alpha)));
            };
        let unit = r / 24.0 * 0.95;
        if k < 1.0 {
            let mic = icons::mic();
            icon(
                mic,
                unit * (1.0 - 0.4 * k.min(1.0)),
                1.0 - k.min(1.0),
                hex(ICON),
                p,
            );
        }
        if k > 0.0 {
            // The check draws on as the circle turns green.
            let check = {
                let c = Outline::new(&icons::check(), 0.05);
                c.trim(0.0, (k.min(1.0)) * c.length())
            };
            let o = check.transform(
                &Affine::translate(-12.0, -12.0)
                    .then(&Affine::scale(unit * 1.05))
                    .then(&Affine::translate(c.x, c.y)),
            );
            p.outline(&o, &StrokeStyle::solid(2.2 * unit, hex(GREEN)));
        }
    }

    /// The dot before the status: none while finishing, a small green disc
    /// with a check once pasted.
    fn draw_status_dot(&self, drop: f32, p: &mut Painter) {
        let dot = Point::new(self.status_dot.x, self.status_dot.y + drop);
        match self.mode {
            Mode::Finishing => {}
            Mode::Done => {
                let c = dot;
                let a = self.status_alpha;
                p.shape(ShapeInstance::circle(
                    c,
                    5.0,
                    Paint::solid(hex(GREEN).fade(a)),
                ));
                let o = Outline::new(&icons::check(), 0.05).transform(
                    &Affine::translate(-12.0, -12.0)
                        .then(&Affine::scale(0.3))
                        .then(&Affine::translate(c.x, c.y)),
                );
                p.outline(&o, &StrokeStyle::solid(1.4, hex(BODY).fade(a)));
            }
            _ => p.shape(ShapeInstance::circle(
                dot,
                3.6,
                Paint::solid(self.status_colour.fade(self.status_alpha)),
            )),
        }
    }
}

/// The kurbo path type, through the framework.
mod kurbo_path {
    pub use vo::scene::path::kurbo::BezPath;
}

impl Popup {
    fn island_mode(&self) -> Mode {
        self.kit_mode().into()
    }

    /// The text is edited, or was and is sent: the card shows it as one
    /// text with the edit caret, until the next recording.
    fn island_editing(&self) -> bool {
        self.edit_view.is_some() || self.edit_sent
    }

    /// Lines the words take in the pill.
    fn pill_lines(&self) -> u32 {
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            return 1;
        }
        let style = pill_style(&self.look);
        let h = self
            .measure
            .layout(&text, &style, Some(pill_text_w(WIDTH)))
            .size
            .h;
        ((h / style.line_height).round() as u32).max(1)
    }

    /// The shape the capsule should have now.
    fn island_geo(&self, mode: Mode, card: bool) -> Geo {
        let w = WIDTH;
        let finishing = matches!(mode, Mode::Finishing) && self.files.fraction.is_some();
        match mode {
            Mode::Loading => Geo {
                w: self.load_w(),
                h: LOAD_H,
                r: LOAD_H / 2.0,
            },
            Mode::Cancelled => Geo {
                w: DOT_SIZE,
                h: DOT_SIZE,
                r: DOT_SIZE / 2.0,
            },
            _ if card => {
                let h = card_h(self.lines, finishing);
                Geo {
                    w,
                    h,
                    r: CARD_RADIUS,
                }
            }
            _ => {
                let h = pill_h(self.pill_lines().min(PILL_LINES), finishing);
                Geo { w, h, r: h / 2.0 }
            }
        }
    }

    pub(crate) fn sync_island(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Island {
            return;
        }
        let mode = self.island_mode();
        if !self.island.card && self.pill_lines() > PILL_LINES {
            self.island.card = true;
        }
        let lines = if self.island.card {
            self.lines
        } else {
            self.pill_lines().min(PILL_LINES)
        };
        let first = self.island.mode.is_none();
        if self.island.mode != Some(mode) {
            if !first {
                self.island.mode_at = t;
            }
            self.island.mode = Some(mode);
        }
        let geo = self.island_geo(mode, self.island.card);
        let text = self.island_status_text(mode);
        let s = &mut self.island;
        if first {
            // Open from a dot.
            s.shape.at = self.shown_at;
        }
        if s.shape.to != geo {
            let now = if first { s.shape.from } else { s.shape.at(t) };
            s.shape = Shape {
                from: now,
                to: geo,
                at: if first { self.shown_at } else { t },
            };
            s.room = s.lines;
            let kind = (
                s.card,
                matches!(mode, Mode::Loading | Mode::Cancelled).then_some(mode),
            );
            if s.kind != Some(kind) {
                s.kind = Some(kind);
                s.fade_at = s.shape.at;
            }
        }
        s.lines = lines;
        if s.status.as_ref().is_some_and(|st| st.state != text.state) {
            s.old = s.status.take();
            s.label_at = t;
        }
        s.status = Some(text);
        let f = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if (f - s.prog.1).abs() > 1e-4 {
            let cur = s.prog.0 + (s.prog.1 - s.prog.0) * ease_out((t - s.prog.2) / 0.35);
            s.prog = (cur, f, t);
        }
        if self.word_count != s.words {
            s.words = self.word_count;
            s.word_at = t;
        }
    }

    /// The status row in `mode`.
    fn island_status_text(&self, mode: Mode) -> StatusText {
        let status = self.status();
        let (state, detail, colour) = match mode {
            Mode::Done => {
                let app = self
                    .files
                    .live
                    .as_ref()
                    .and_then(|d| d.target.as_ref())
                    .map(|t| t.app.clone());
                match app {
                    Some(app) if !app.is_empty() => (
                        self.delivery_word().to_string(),
                        format!("  ·  into {app}"),
                        hex(GREEN),
                    ),
                    _ => (self.delivery_word().to_string(), String::new(), hex(GREEN)),
                }
            }
            Mode::Cancelled => ("Cancelled".to_string(), String::new(), hex(DIM)),
            _ => (status.state.clone(), status.detail.clone(), hex(STATE)),
        };
        StatusText {
            state,
            detail,
            colour,
            finishing: mode == Mode::Finishing && self.files.fraction.is_some(),
            done: mode == Mode::Done,
        }
    }

    fn island_busy(&self, t: f64) -> bool {
        let s = &self.island;
        !self.closing
            && (t < s.shape.at + MORPH_SECS + CONTENT_DELAY + CONTENT_FADE
                || t < s.label_at + SWAP_OUT + SWAP_IN)
    }

    pub(crate) fn arm_island_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.look.theme == OsdTheme::Island
            && self.extra.tick.is_none()
            && self.island_busy(cx.t())
        {
            self.extra.tick = Some(cx.after(1.0 / TICK_HZ, Msg::Tick));
        }
    }

    pub(crate) fn island_surface(_look: &Look) -> Size {
        Size::new(
            (WIDTH + 2.0 * MARGIN_X).ceil(),
            (card_h(CARD_LINES, true) + 2.0 * MARGIN_Y).ceil(),
        )
    }

    pub(crate) fn island_view(&self) -> Element<Msg> {
        let size = Self::island_surface(&self.look);
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        root.child(self.island_panel(size, 0.0, 0.0))
    }

    /// The whole look in a `size` box at `(x, y)`.
    fn island_panel(&self, size: Size, x: f32, y: f32) -> Element<Msg> {
        let l = &self.look;
        let s = &self.island;
        let t = self.extra.now;
        let mode = s.mode.unwrap_or(Mode::Loading);
        let card = s.card && !matches!(mode, Mode::Loading | Mode::Cancelled);
        let g = s.shape.to;
        let ox = (size.w - g.w) / 2.0;
        let oy = MARGIN_Y;
        let status = self.status();
        let finishing = mode == Mode::Finishing && self.files.fraction.is_some();
        // The content of a new mode or form fades in once the shape is
        // nearly there.
        let content_at = s.fade_at + CONTENT_DELAY;
        // The capsule now, and how far its bottom edge is above where it
        // is going; the status row rides on that edge, as the canvas's
        // rules, bar and status dot do.
        let gm = s.shape.at(t);
        let drop = gm.h - g.h;
        let k = if reduced() {
            1.0
        } else {
            ease_out((t - content_at) / CONTENT_FADE)
        };

        // Where the text and the status go, relative to the capsule.
        let (text_x, text_y, text_w, line_h) = if card {
            (
                CARD_PAD_X,
                CARD_HEADER + 15.0,
                WIDTH - 2.0 * CARD_PAD_X,
                CARD_LINE,
            )
        } else {
            (PILL_TEXT_X, PILL_PAD_TOP, pill_text_w(WIDTH), PILL_LINE)
        };
        let lines = if card {
            self.lines
        } else {
            self.pill_lines().min(PILL_LINES)
        };
        let text_bottom = text_y + lines as f32 * line_h;
        let (bar, status_y) = if card {
            let rule = text_bottom + 13.0;
            let bar = Rect::new(CARD_PAD_X, rule + 16.0, WIDTH - 2.0 * CARD_PAD_X, 5.0);
            let sy = if finishing { bar.y + 13.0 } else { rule + 14.0 };
            (bar, sy)
        } else {
            let bar = Rect::new(text_x, text_bottom + 6.0, text_w - 6.0, 6.0);
            let sy = if finishing {
                text_bottom + BAR_ROW + 2.0
            } else {
                text_bottom + 3.0
            };
            (bar, sy)
        };
        let done = mode == Mode::Done;
        let text = self.island_status_text(mode);
        // A new status label waits for the old one to go, so the two never
        // overlap; the dot before it comes in with it.
        let since = t - s.label_at;
        let swap_in = if reduced() {
            1.0
        } else {
            ease_out((since - SWAP_OUT) / SWAP_IN)
        };
        let status_x = if card { CARD_PAD_X } else { text_x };
        let status_line = STATUS_H;
        let rules = if card {
            vec![(CARD_HEADER, false), (text_bottom + 13.0, true)]
        } else {
            Vec::new()
        };

        // The caret after the newest word, in the card. While editing the
        // edit caret, in the same blue, takes its place (`IslandSite`).
        let caret = if card && mode == Mode::Listening && !self.island_editing() {
            let style = card_style(l);
            let room = self.island_room(text_w, &style);
            self.island_last_line(&self.shown[..room], text_w, &style)
                .map(|(ly, right)| {
                    (
                        Point::new(text_x + right + 4.0, text_y + ly + 4.0),
                        s.word_at,
                    )
                })
        } else {
            None
        };

        let scene = Scene {
            surface: size,
            shape: s.shape,
            mode,
            mode_at: s.mode_at,
            card,
            shown_at: self.shown_at,
            content_at,
            levels: self.exhale_levels(&self.wave),
            rest: self.exhale(),
            newest_at: self.newest_at,
            finish_at: self.extra.finish_at,
            prog: finishing.then_some(s.prog),
            bar,
            status_dot: Point::new(status_x + 4.0, status_y + status_line / 2.0),
            status_colour: if done {
                hex(GREEN)
            } else if mode == Mode::Cancelled {
                hex(DIM)
            } else {
                hex(BLUE)
            },
            status_alpha: swap_in,
            caret,
            rules,
            words_at: if !self.motion("word-bulge") || self.still() {
                Vec::new()
            } else {
                self.tally
                    .arrived
                    .iter()
                    .map(|a| a.1)
                    .filter(|&at| t - at < BULGE_SECS)
                    .collect()
            },
            still: self.still(),
        };
        let until = scene.until(t);
        let canvas = at(
            Element::canvas("capsule", until, move |t, rect, p| scene.draw(t, rect, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h);

        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.9)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let mut panel = at(Element::layer("island"), x, y)
            .size(size.w, size.h)
            .presence(Transition::fade(), exit)
            .child(canvas);
        if matches!(mode, Mode::Loading) {
            return panel.child(self.island_loading(ox, oy, g.w, k, &status));
        }
        if mode == Mode::Cancelled {
            return panel;
        }
        let f = fonts(&l.fonts);
        let opacity = k;
        // The clock sits after the dot, until the stop square replaces both.
        if mode != Mode::Finishing {
            let (cx, cy) = if card {
                (46.0, CARD_HEADER / 2.0)
            } else {
                (46.0, gm.h / 2.0)
            };
            let style = TextStyle::new(f.text, 14.0, 18.0).tracking(0.3);
            panel = panel.child(
                at(
                    self.exhale_motion(
                        digit_roll(
                            "clock",
                            &self.clock,
                            style,
                            self.exhale_color(hex(WHITE), hex(DIM)),
                            Spring::critical_to_98(0.25),
                        ),
                        &[Prop::TextColor],
                    ),
                    ox + cx,
                    oy + cy - 9.0,
                )
                .opacity(opacity),
            );
            // The pace under the clock, its digits leaning as you speed up
            // and standing up when you pause (MO-66).
            let small = TextStyle::new(f.medium, 9.5, 12.0).tracking(0.3);
            let (wpm, lean) = self.tally_pace();
            let cell = self.measure.layout("0", &small, None).size.w;
            let pace_on = self.motion("lean-pace");
            panel = if !pace_on {
                panel
            } else {
                panel.child(
                    at(
                        Element::row("pace")
                            .align_items(AlignItems::CENTER)
                            .child(tally::lean_digits(
                                "wpm",
                                &wpm.to_string(),
                                small,
                                hex(DIM),
                                cell,
                                lean,
                            ))
                            .child(
                                Element::label("unit", " wpm")
                                    .text_style(small)
                                    .color(hex(HINT)),
                            ),
                        ox + cx,
                        oy + cy + 10.0,
                    )
                    .opacity(opacity),
                )
            };
        }
        panel =
            panel.child(self.island_words(card, ox + text_x, oy + text_y, text_w, lines, opacity));
        // While finishing the status is larger and has no dot, as in 03-finish.
        let row = |st: &StatusText, a: f32| {
            let (status_px, state_colour, lead) = if st.finishing {
                (13.6, hex(0xb9c3d6), 0.0)
            } else {
                (STATUS_PX, st.colour, 13.0)
            };
            let style = TextStyle::new(f.medium, status_px, status_line).tracking(0.1);
            let row = Element::row(format!("status-{}", st.state))
                .align_items(AlignItems::CENTER)
                .height(status_line)
                .child(
                    Element::label("state", st.state.as_str())
                        .text_style(style)
                        .color(state_colour)
                        .motion(Prop::TextColor, Spring::critical_to_98(0.25)),
                )
                .child(
                    Element::label("detail", st.detail.replace("  ·  ", "   ·   "))
                        .text_style(style)
                        .color(if st.done {
                            hex(GREEN).fade(0.85)
                        } else {
                            hex(HINT)
                        }),
                );
            at(row, ox + status_x + lead, oy + status_y + drop)
                .opacity(opacity * a)
                .presence(Transition::NONE, Transition::NONE)
                .motion(Prop::Position, Motion::Snap)
                .motion(Prop::Opacity, Motion::Snap)
        };
        if reduced() {
            return panel.child(row(&text, 1.0));
        }
        if let Some(old) = s.old.as_ref().filter(|_| since < SWAP_OUT) {
            panel = panel.child(row(old, 1.0 - ease_out(since / SWAP_OUT)));
        }
        panel.child(row(&text, swap_in))
    }

    fn loading_style(&self) -> TextStyle {
        let f = fonts(&self.look.fonts);
        TextStyle::new(f.text, 13.0, 18.0).tracking(0.1)
    }

    fn loading_label(status: &StatusLine) -> String {
        if status.detail.trim().is_empty() {
            "Loading model".to_string()
        } else {
            format!(
                "{} {}",
                status.state,
                status.detail.trim_start_matches("  ·  ")
            )
        }
    }

    /// The loading pill's width: the spinner, the label, room for the
    /// forming cue and the percentage. In steps of `LOAD_STEP`, so the
    /// seconds counting up do not move its edge each time.
    fn load_w(&self) -> f32 {
        let label = Self::loading_label(&self.status());
        let lw = self
            .measure
            .layout(&label, &self.loading_style(), None)
            .size
            .w;
        let w = 42.0 + lw + 4.0 + LOAD_CUE + LOAD_PCT;
        ((w / LOAD_STEP).ceil() * LOAD_STEP).clamp(LOAD_W, WIDTH)
    }

    /// The loading pill: the spinner, "Loading" and the percentage.
    fn island_loading(
        &self,
        ox: f32,
        oy: f32,
        w: f32,
        k: f32,
        status: &StatusLine,
    ) -> Element<Msg> {
        let style = self.loading_style();
        let pct = self
            .files
            .fraction
            .map(|f| format!("{}%", (f.clamp(0.0, 1.0) * 100.0).round() as u32))
            .unwrap_or_default();
        let label = Self::loading_label(status);
        // The forming cue after the label, as far as the percentage. Its
        // place stays free while there is none yet: the percentage rolls in
        // there a moment later.
        let from = ox + 42.0 + self.measure.layout(&label, &style, None).size.w + 4.0;
        let to = ox + w - LOAD_PCT;
        let tail = model_signals::TailText {
            style,
            width: (to - from).max(0.0),
            rows: 1,
        };
        let layer = Element::layer("loading")
            .child(at(
                Element::label("load", label)
                    .text_style(style)
                    .color(hex(WHITE)),
                ox + 42.0,
                oy + LOAD_H / 2.0 - 9.0,
            ))
            .child(at(
                digit_roll("pct", &pct, style, hex(STATE), Spring::critical_to_98(0.25)),
                ox + w - LOAD_PCT + 4.0,
                oy + LOAD_H / 2.0 - 9.0,
            ));
        // The cue shows from the voice onset on, ahead of the label's fade in.
        let cue = self.signal_marks_in(
            Element::layer("loading-cue"),
            &[],
            &tail,
            (from, oy + LOAD_H / 2.0 - 9.0),
            hex(WHITE),
        );
        Element::layer("loading-pill")
            .child(layer.opacity(k))
            .child(cue)
    }

    /// How many shown words have room in the capsule now. While it grows
    /// for a new line, that line's words wait until its bottom edge, with
    /// the status row on it, is within `ROOM_GAP` of its place, so they
    /// never draw over the status row.
    fn island_room(&self, w: f32, style: &TextStyle) -> usize {
        let s = &self.island;
        let all = self.shown.len();
        // While editing every window word draws, or the caret could sit past
        // the last drawn word.
        if reduced()
            || self.edit_view.is_some()
            || s.shape.at(self.extra.now).h >= s.shape.to.h - ROOM_GAP
        {
            return all;
        }
        let fits = |n: usize| {
            let text = self.shown[..n]
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let h = self.measure.layout(&text, style, Some(w)).size.h;
            (h / style.line_height).round() as u32 <= s.room
        };
        (0..=all).rev().find(|&n| fits(n)).unwrap_or(0)
    }

    /// The words: in the pill all white; in the card the older lines dim and
    /// the newest line white.
    fn island_words(
        &self,
        card: bool,
        x: f32,
        y: f32,
        w: f32,
        lines: u32,
        opacity: f32,
    ) -> Element<Msg> {
        let l = &self.look;
        // The words come back at full ink or not at all: a fade from 0 as the
        // capsule changes form left the reading line at 20 % ink (RD-09), and
        // a new word's own fade in on top of a partly faded text went under
        // the 70 % floor too. A form change still fades the old words out
        // through `presence`.
        let opacity = if opacity <= 0.0 { 0.0 } else { 1.0 };
        let live = |key: String, words: &[Word], style: TextStyle, colour: Color| {
            Element::live_text(key, words)
                .text_style(style)
                .revision_motion(revision::motion(&self.look))
                .word_revise(revision::revise(&self.look))
                .color(colour)
                .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                .motion(Prop::Offset, Spring::POP)
                .word_enter(self.ghost_enter(Transition::fade().offset(0.0, 5.0)))
        };
        if !card {
            let style = pill_style(l);
            let shown = &self.shown[..self.island_room(w, &style)];
            let text = Element::column("pilltext")
                .width(w)
                .height(lines as f32 * PILL_LINE)
                .child(self.island_handoff(
                    live("words".into(), shown, style, hex(WHITE)),
                    &style,
                    w,
                ));
            let tail = model_signals::TailText {
                style,
                width: w,
                rows: lines,
            };
            let text = self.signal_marks_in(text, shown, &tail, (0.0, 0.0), hex(WHITE));
            return at(
                self.with_hearing(text, &style, w, hex(WHITE), hex(BLUE)),
                x,
                y,
            )
            .opacity(opacity)
            .presence(Transition::NONE, Transition::fade())
            .motion(Prop::Opacity, Motion::Snap);
        }
        let style = card_style(l);
        let words = &self.shown[..self.island_room(w, &style)];
        let (old, new) = words.split_at(self.island_split(words, w, &style));
        let new_y = self
            .island_last_line(words, w, &style)
            .map_or(0.0, |(ly, _)| ly);
        let mut text = Element::column("cardtext")
            .width(w)
            .height(lines as f32 * CARD_LINE);
        if self.delivered().is_some() {
            // The hand-off lays out all the words itself, in one block.
            text = text.child(self.island_handoff(Element::column("handed"), &style, w));
        } else if self.island_editing() {
            // While editing and after it the text is one text in white, in
            // the node the older lines had, so no line fades in from
            // nothing, nor when the edit ends.
            text = text.child(at(
                live("old".into(), words, style, hex(WHITE))
                    .word_enter(Transition::NONE)
                    .width(w),
                0.0,
                0.0,
            ));
        } else {
            text = text
                .child(at(
                    live("old".into(), old, style, hex(DIM))
                        .word_enter(Transition::NONE)
                        .width(w),
                    0.0,
                    0.0,
                ))
                // A node per line: when a line fills, its words go dim in
                // "old" at once and the next line starts in a fresh node,
                // so nothing slides or fades between the two lines.
                .child(at(
                    live(format!("new-{}", old.len()), new, style, hex(WHITE)).width(w),
                    0.0,
                    new_y,
                ));
            let tail = model_signals::TailText {
                style,
                width: w,
                rows: lines,
            };
            text = self.signal_marks_in(text, words, &tail, (0.0, 0.0), hex(WHITE));
        }
        at(
            self.with_hearing(text, &style, w, hex(WHITE), hex(BLUE)),
            x,
            y,
        )
        .opacity(opacity)
        .presence(Transition::NONE, Transition::fade())
        .motion(Prop::Opacity, Motion::Snap)
    }

    /// The words, or while a delivery shows the text leaving for the app and
    /// folding into the receipt (MO-42, MO-43). `style` and `w` are the ones
    /// `words` is laid out with.
    fn island_handoff(&self, words: Element<Msg>, style: &TextStyle, w: f32) -> Element<Msg> {
        self.handoff_or(words, style, Some(w), hex(WHITE), hex(GREEN))
    }

    /// The index of the first word on the last line of the card's text.
    fn island_split(&self, words: &[Word], w: f32, style: &TextStyle) -> usize {
        let text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let shaped = self.measure.layout(&text, style, Some(w));
        let Some(last_y) = shaped.glyphs.iter().map(|g| g.pos.y).reduce(f32::max) else {
            return 0;
        };
        let mut start = 0usize;
        for (i, word) in words.iter().enumerate() {
            let range = start..start + word.text.len();
            start = range.end + 1;
            if let Some(g) = shaped
                .glyphs
                .iter()
                .find(|g| range.contains(&(g.cluster as usize)))
            {
                if (g.pos.y - last_y).abs() < 1.0 {
                    return i;
                }
            }
        }
        words.len()
    }

    /// The last line's top and its right end in the card's text.
    fn island_last_line(&self, words: &[Word], w: f32, style: &TextStyle) -> Option<(f32, f32)> {
        let text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let shaped = self.measure.layout(&text, style, Some(w));
        let last_y = shaped.glyphs.iter().map(|g| g.pos.y).reduce(f32::max)?;
        let row = (last_y / style.line_height).floor();
        let tail = words[self.island_split(words, w, style)..]
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let right = self.measure.layout(&tail, style, None).size.w;
        Some((row * style.line_height, right))
    }
}

/// The pill's height with `lines` lines of words.
fn pill_h(lines: u32, bar: bool) -> f32 {
    PILL_PAD_TOP
        + lines as f32 * PILL_LINE
        + 3.0
        + STATUS_H
        + PILL_PAD_BOTTOM
        + if bar { BAR_ROW } else { 0.0 }
}

/// The card's height with `lines` lines of words.
fn card_h(lines: u32, bar: bool) -> f32 {
    CARD_HEADER
        + 15.0
        + lines as f32 * CARD_LINE
        + 13.0
        + CARD_STATUS
        + if bar { 14.0 } else { 0.0 }
}

/// Island's edit adapter (plan section 4.5): the capsule keeps its form,
/// so in the pill the window is the pill's lines in the pill's text box,
/// and in the card the card's. The card's blue caret is the edit caret.
/// Its word bulge and lean pace are in the quiet list.
pub(crate) struct IslandSite;

impl edit_site::EditSite for IslandSite {
    fn text_box_in(&self, p: &Popup) -> edit_site::TextBox {
        if p.island.card {
            edit_site::TextBox {
                style: card_style(&p.look),
                width: text_width(&p.look),
            }
        } else {
            edit_site::TextBox {
                style: pill_style(&p.look),
                width: pill_text_w(WIDTH),
            }
        }
    }

    fn edit_lines_in(&self, p: &Popup) -> u32 {
        if p.island.card {
            CARD_LINES
        } else {
            PILL_LINES
        }
    }

    fn caret(&self, _look: &Look) -> edit_site::CaretStyle {
        edit_site::CaretStyle {
            caret: vo::scene::flow::Caret {
                color: Some(hex(BLUE)),
                ..vo::scene::flow::Caret::default()
            },
            selection: Color::hex(BLUE, 0.35),
            mark: hex(BLUE),
        }
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use super::{hex, BLUE, PILL_LINES};
    use vo_core::edit::EditAction;
    use voxtype::osd::config::OsdTheme;

    /// M7.L5: Island edits in its own layout, in the card. With the caret
    /// on the newest, a middle and the top line, at both widths: the caret
    /// on its glyph within 1 px in every frame, the popup's size unchanged,
    /// and the golden `tests/fixtures/osd-vo/edit-island.txt`.
    #[test]
    fn island_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Island, "island");
    }

    /// M7.L5: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn island_edits_within_the_envelope() {
        envelope(OsdTheme::Island, "island");
    }

    /// The card's blue caret gives way to the edit caret in the same blue.
    #[test]
    fn island_edits_with_one_blue_caret() {
        let mut r = Rig::new(OsdTheme::Island, false);
        // The card's caret shows while listening.
        r.p.files.recording = true;
        r.dictate(160);
        // A word just came: the caret is solid, not in its slow blink.
        r.p.island.word_at = 0.9;
        r.show(0.9);
        assert_eq!(r.carets(hex(BLUE)), 1, "the card's caret");
        r.enter(1.0);
        assert_eq!(r.carets(hex(BLUE)), 1, "the edit caret only");
    }

    /// In the pill the capsule keeps its form: the window is the pill's
    /// lines, at most `PILL_LINES`, in the pill's text box, and the caret
    /// stands on its glyph.
    #[test]
    fn island_edits_in_the_pill() {
        let mut r = Rig::new(OsdTheme::Island, false);
        r.dictate(8);
        assert!(!r.p.island.card, "a short text is in the pill");
        let before = r.size();
        r.enter(1.0);
        assert!(!r.p.island.card, "the pill stays");
        assert_eq!(r.size(), before);
        r.caret_follows(1.0, 1.5, "pill, entered");
        let more = " more words to fill the pill";
        r.act(EditAction::Insert(more.into()), 1.5);
        r.caret_follows(1.5, 2.1, "pill, typed");
        let v = r.p.edit_view.as_ref().unwrap();
        assert!(v.lines <= PILL_LINES, "{} lines", v.lines);
        assert!(!r.p.island.card);
    }
}
