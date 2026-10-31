//! The `captions` look of the vo popup, after mock-up 13: the words as
//! large cinema subtitles in two centred lines on a soft dark backdrop near
//! the bottom of the screen. The newest word is in the accent blue; when a
//! line fills, the lines roll up and the oldest fades out above. A tiny red
//! dot pulses beside the newest line while the mic is live, a word the
//! engine is unsure of gets a thin blue underline, and a short progress line
//! with a small status shows under the text while the final pass runs.
//!
//! The mock-up also lights the whole screen edge while recording. That is
//! a second surface, fullscreen and click-through, under the band: a thin
//! blue line round the output whose glow follows the voice. It fades out
//! when the mic stops and the surface closes.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).
//!
//! While the popup is open it keeps a little idle life (pcn, RD-02): the
//! backdrop's flat top and bottom strips, above and below the newest line,
//! act as the letterbox bars. They carry a faint film grain (specks at 3 %
//! alpha) and a projector flicker that dims them by at most 2 %, stepped 12
//! times a second. Neither runs once the popup closes or under reduced
//! motion.
//!
//! Round 2 (R2-13) adds three opt-in features:
//!
//! - `liquid-echo`: a faint water surface in the strip above the top line;
//!   each new word sends a flat ring out from its x there, at most 20 %
//!   alpha, gone in 1 s. Reduced motion: the surface without rings.
//! - `prism-capture`: in the same strip a thin beam meets a glass slab and
//!   fans out into a fine spectrum with a little dust, the fan wider and
//!   brighter with the voice, at most 25 % alpha. Reduced motion: a still
//!   fan whose brightness follows the voice.
//! - `typewriter-set`: a ticking ruler under the newest line with a carriage
//!   marker that glides to the end of the newest word, and in a pause a thin
//!   box round the newest phrase. The tick is drawn only, never heard.
//!   Reduced motion: the marker jumps.
//!
//! While editing ([`CaptionsSite`]) both lines are full size and flat, the
//! newest word is part of its line, and a line keeps its left edge while
//! you type (the sticky centring of `EditView::centre`).

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, Contour, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

const WHITE: u32 = 0xf4f5fa;
const ACCENT: u32 = 0x6f88ff;
const UNDER: u32 = 0x3f7bff;
const RED: u32 = 0xff4d5e;
const HINT: u32 = 0x9aa3bb;
const BLUE: u32 = 0x3b82ff;
const GREEN: u32 = 0x46d98a;
const BACKDROP: u32 = 0x04071a;
/// The screen-edge light, between the mock-up's blue and violet sides.
const EDGE: u32 = 0x5a6bff;

fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

/// The band: wide enough for the mock-up's 1100 px lines and the backdrop's
/// soft edge.
const BAND_W: f32 = 1240.0;
const BAND_H: f32 = 196.0;
const TEXT_PX: f32 = 36.0;
const LINE: f32 = 54.0;
const WRAP: f32 = 1090.0;
pub(crate) const MAX_LINES: u32 = 2;
/// Line centres by age, 0 the newest, with opacity.
const SLOTS: [(f32, f32); 3] = [(108.0, 1.0), (54.0, 0.75), (0.0, 0.0)];
const ROLL_SECS: f64 = 0.3;
const STATUS_Y: f32 = 160.0;
const TICK_HZ: f64 = 60.0;
/// MO-33: the fold's spring; it nears edge-on in about 220 ms.
const FOLD_SECS: f64 = 0.22;

/// R2-13-a: a ring's life, its reach and its alpha.
pub(crate) const LIQUID_ECHO: &str = "liquid-echo";
const ECHO_SECS: f64 = 1.0;
const ECHO_REACH: f32 = 70.0;
const ECHO_ALPHA: f32 = 0.2;
/// R2-13-b: the fan's length at rest and at full voice, its alpha at full
/// voice, its colours and dust.
pub(crate) const PRISM_CAPTURE: &str = "prism-capture";
const FAN_LEN: (f32, f32) = (160.0, 460.0);
const FAN_ALPHA: f32 = 0.25;
const FAN_COLOURS: [u32; 7] = [
    0xff5a6e, 0xff9a4a, 0xffe35a, 0x5ae68a, 0x4ac8ff, 0x6f88ff, 0xa66bff,
];
const FAN_DUST: u32 = 14;
/// R2-13-a and R2-13-b: the strip over the top line is at most this tall.
const SKY_H: f32 = 40.0;
/// R2-13-c: the ruler's tick pitch, the marker's glide and how long a
/// pause is before the phrase box shows; words this close together are
/// one phrase.
pub(crate) const TYPEWRITER_SET: &str = "typewriter-set";
const RULER_TICK: f32 = 12.0;
const CARRIAGE_SECS: f64 = 0.18;
const PHRASE_PAUSE: f64 = 0.6;
const PHRASE_GAP: f64 = 0.45;

#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    ui: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    // Added to the registry once (`FontRegistry::system`), so every
    // registry, such as a test's, has its own ids.
    let add = |pattern: &str, or: u32| reg.system(pattern).unwrap_or(or);
    Fonts {
        text: add("Nunito:medium", SANS),
        ui: add("Nunito", SANS),
    }
}

fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, LINE).tracking(-0.1)
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

/// State only the captions look keeps.
pub struct State {
    mode: Option<Mode>,
    mode_at: f64,
    prog: (f32, f32, f64),
    /// The backdrop's box: from, to, when.
    back: (Rect, Rect, f64),
}

impl Default for State {
    fn default() -> State {
        let r = Rect::new(BAND_W / 2.0, SLOTS[0].0, 0.0, 0.0);
        State {
            mode: None,
            mode_at: f64::NEG_INFINITY,
            prog: (0.0, 0.0, 0.0),
            back: (r, r, f64::NEG_INFINITY),
        }
    }
}

/// What the canvas draws: the backdrop, the live dot, the underlines, the
/// progress line and the check.
#[derive(Clone)]
struct Scene {
    mode: Mode,
    mode_at: f64,
    shown_at: f64,
    /// The backdrop's box, eased from its last size: from, to, when.
    back: (Rect, Rect, f64),
    dot: Option<Point>,
    /// Thin underlines: x, y, width.
    doubts: Vec<(f32, f32, f32)>,
    /// The progress line: box and eased fraction.
    prog: Option<(Rect, (f32, f32, f64))>,
    check: Option<Point>,
    /// pcn: the idle grain and flicker run (open, listening, not reduced).
    idle: bool,
    /// R2-13-a and R2-13-b: what the strip above the top line holds.
    sky: Option<Sky>,
}

/// R2-13-a and R2-13-b: the strip above the top line, its rings and fan.
#[derive(Clone, Debug)]
struct Sky {
    /// The strip, in the band's pixels.
    strip: Rect,
    /// R2-13-a: each new word's x and when it came; `None` when off.
    rings: Option<Vec<(f32, f64)>>,
    /// R2-13-b: the voice level now (0 to 1); `None` when off.
    prism: Option<f32>,
    still: bool,
}

impl Sky {
    fn until(&self, now: f64) -> f64 {
        let rings = self
            .rings
            .iter()
            .flatten()
            .map(|r| r.1 + ECHO_SECS)
            .fold(now, f64::max);
        if self.prism.is_some() {
            now + 0.25
        } else {
            rings
        }
    }

    fn draw(&self, t: f64, p: &mut Painter) {
        let s = self.strip;
        let mid = s.y + s.h * 0.55;
        if let Some(rings) = &self.rings {
            // The water: three faint lines that close up toward the
            // horizon, as a tilted surface does.
            for (i, f) in [0.35f32, 0.65, 0.88].iter().enumerate() {
                let y = s.y + s.h * f;
                p.shape(ShapeInstance::line(
                    Point::new(s.x, y),
                    Point::new(s.x + s.w, y),
                    1.0,
                    Paint::linear(
                        Point::new(s.x, y),
                        hex(EDGE).fade(0.0),
                        Point::new(s.x + s.w / 2.0, y),
                        hex(EDGE).fade(0.05 + 0.03 * i as f32),
                    ),
                ));
            }
            if !self.still {
                for &(x, born) in rings {
                    for j in 0..2 {
                        let u = ((t - born) / ECHO_SECS) as f32 - 0.2 * j as f32;
                        if !(0.0..1.0).contains(&u) {
                            continue;
                        }
                        let rx = 6.0 + ECHO_REACH * ease_out(u as f64);
                        let ry = (rx * 0.16).min(s.h * 0.45);
                        let a = ECHO_ALPHA * (1.0 - u) * (1.0 - u);
                        let pts: Vec<Point> = (0..=48)
                            .map(|k| {
                                let a = std::f32::consts::TAU * k as f32 / 48.0;
                                Point::new(x + rx * a.cos(), mid + ry * a.sin())
                            })
                            .filter(|q| q.x > s.x && q.x < s.x + s.w)
                            .collect();
                        if pts.len() > 2 {
                            p.stroke(
                                &Contour::polyline(&pts, false),
                                &StrokeStyle::solid(1.2, hex(ACCENT).fade(a)).glow(3.0),
                            );
                        }
                    }
                }
            }
        }
        if let Some(level) = self.prism {
            let grow = if self.still { 0.5 } else { level };
            let len = FAN_LEN.0 + (FAN_LEN.1 - FAN_LEN.0) * grow;
            let a = FAN_ALPHA * (0.25 + 0.75 * level);
            let slab = Point::new(s.x + 90.0, mid);
            // The beam in, and the slab it meets.
            p.shape(ShapeInstance::line(
                Point::new(s.x + 10.0, mid),
                slab,
                1.2,
                Paint::linear(
                    Point::new(s.x + 10.0, mid),
                    Color::WHITE.fade(0.0),
                    slab,
                    Color::WHITE.fade(a * 1.6),
                ),
            ));
            let half = (s.h * 0.5 - 2.0).max(2.0);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(slab.x - 1.5, mid - half, 3.0, 2.0 * half), 1.0),
                Paint::solid(Color::WHITE.fade(a)),
            ));
            // The fan: a spectrum spreading to the strip's height.
            let n = FAN_COLOURS.len();
            for (i, c) in FAN_COLOURS.iter().enumerate() {
                let f = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
                let end = Point::new((slab.x + len).min(s.x + s.w), mid + f * half);
                p.shape(ShapeInstance::line(
                    slab,
                    end,
                    1.4,
                    Paint::linear(slab, hex(*c).fade(a), end, hex(*c).fade(0.0)),
                ));
            }
            if !self.still {
                for i in 0..FAN_DUST {
                    let u =
                        (noise(i * 37 + 1) + 0.12 * t as f32 * (0.5 + noise(i * 11 + 4))).fract();
                    let f = noise(i * 23 + 9) * 2.0 - 1.0;
                    let q = Point::new(slab.x + u * len, mid + f * half * u);
                    if q.x < s.x + s.w {
                        p.shape(ShapeInstance::circle(
                            q,
                            0.8,
                            Paint::solid(Color::WHITE.fade(a * (1.0 - u))),
                        ));
                    }
                }
            }
        }
    }
}

/// pcn: the idle motion's frame rate; the grain's alpha (3 % white as an
/// sRGB blend would show it: the GPU blends in linear light, where a 3 %
/// speck lifts this dark backdrop about 40 levels instead of 8), specks per
/// bar and speck radius; the flicker's deepest dip, as the alpha of a black
/// veil (a white one blends in linear light and lifts the dark backdrop far
/// more than its alpha).
const IDLE_FPS: f64 = 12.0;
const GRAIN_ALPHA: f32 = 0.006;
const GRAIN_SPECKS: u32 = 24;
const GRAIN_R: f32 = 0.8;
const FLICKER: f32 = 0.02;

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// pcn: the letterbox bars inside backdrop `b`: its flat stretch (between
/// the round ends) above and below the newest line's box, a pixel clear of
/// it.
fn bars(b: Rect) -> [Rect; 2] {
    let (top, bottom) = (SLOTS[0].0 - LINE / 2.0 - 1.0, SLOTS[0].0 + LINE / 2.0 + 1.0);
    let (x, w) = (b.x + b.h / 2.0, (b.w - b.h).max(0.0));
    [
        Rect::new(x, b.y, w, (top - b.y).max(0.0)),
        Rect::new(x, bottom, w, (b.y + b.h - bottom).max(0.0)),
    ]
}

/// pcn: one idle step `n` of grain and flicker on the letterbox bars.
fn draw_film(b: Rect, n: u32, p: &mut Painter) {
    let dip = FLICKER * noise(n.wrapping_mul(31));
    for (i, bar) in bars(b).into_iter().enumerate() {
        if bar.w < 1.0 || bar.h < 2.0 * GRAIN_R {
            continue;
        }
        p.shape(ShapeInstance::rrect(
            RRect::new(bar, 0.0),
            Paint::solid(Color::BLACK.fade(dip)),
        ));
        let seed = n.wrapping_mul(7919) ^ (i as u32).wrapping_mul(0x51ED_270B);
        for k in 0..GRAIN_SPECKS {
            let x = bar.x + noise(seed ^ k.wrapping_mul(0x2545_F491)) * bar.w;
            let v = noise(seed.wrapping_add(k).wrapping_mul(0x6C8E_9CF5));
            let y = bar.y + GRAIN_R + v * (bar.h - 2.0 * GRAIN_R);
            p.shape(ShapeInstance::circle(
                Point::new(x, y),
                GRAIN_R,
                Paint::solid(Color::WHITE.fade(GRAIN_ALPHA)),
            ));
        }
    }
}

fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    Rect::new(
        a.x + (b.x - a.x) * k,
        a.y + (b.y - a.y) * k,
        a.w + (b.w - a.w) * k,
        a.h + (b.h - a.h) * k,
    )
}

impl Scene {
    fn until(&self, now: f64) -> f64 {
        let sky = self.sky.as_ref().map_or(now, |s| s.until(now));
        self.until_base(now).max(sky)
    }

    fn until_base(&self, now: f64) -> f64 {
        let back = self.back.2 + 0.4;
        match self.mode {
            Mode::Listening | Mode::Loading => (now + 0.25).max(back),
            Mode::Finishing => (now + 0.4).max(back),
            _ => (now + 0.2).max(back).max(self.mode_at + 0.5),
        }
    }

    fn draw(&self, t: f64, _rect: Rect, p: &mut Painter) {
        let k = ease_out((t - self.back.2) / 0.3);
        let b = lerp_rect(self.back.0, self.back.1, k);
        // A soft dark backdrop: one wide shadow under a faint fill.
        let rr = RRect::new(b, b.h / 2.0);
        p.shape(ShapeInstance::rrect(
            rr,
            Paint::shadow(hex(BACKDROP).fade(0.62), Point::ZERO, 18.0, -6.0),
        ));
        p.shape(ShapeInstance::rrect(
            rr,
            Paint::solid(hex(BACKDROP).fade(0.22)),
        ));
        if self.idle {
            draw_film(b, (t * IDLE_FPS).floor() as i64 as u32, p);
        }
        if let Some(sky) = &self.sky {
            sky.draw(t, p);
        }
        if let Some(c) = self.dot {
            let pulse = if reduced() {
                0.5
            } else {
                (0.5 - 0.5 * ((t - self.shown_at) * std::f64::consts::TAU).cos()) as f32
            };
            let r = 4.2 * (1.0 + 0.15 * pulse);
            p.shape(ShapeInstance::circle(
                c,
                r,
                Paint::glow(hex(RED).fade(0.4 + 0.4 * pulse), 6.0, 0.7),
            ));
            p.shape(ShapeInstance::circle(c, r, Paint::solid(hex(RED))));
        }
        for &(x, y, w) in &self.doubts {
            let r = Rect::new(x, y, w, 2.0);
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.0),
                Paint::glow(hex(UNDER).fade(0.7), 4.0, 0.6),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(r, 1.0),
                Paint::solid(hex(UNDER)),
            ));
        }
        if let Some((bar, (a, b, at))) = self.prog {
            let f = a + (b - a) * ease_out((t - at) / 0.35);
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, bar.h / 2.0),
                Paint::solid(Color::WHITE.fade(0.12)),
            ));
            let fill = Rect::new(bar.x, bar.y, (bar.w * f).max(bar.h), bar.h);
            p.shape(ShapeInstance::rrect(
                RRect::new(fill, bar.h / 2.0),
                Paint::glow(hex(BLUE).fade(0.6), 4.0, 0.6),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(fill, bar.h / 2.0),
                Paint::linear(
                    Point::new(bar.x, 0.0),
                    hex(BLUE),
                    Point::new(bar.x + bar.w, 0.0),
                    hex(0x8b5cf6),
                ),
            ));
        }
        if let Some(c) = self.check {
            let k = ease_out((t - self.mode_at) / 0.3);
            p.shape(ShapeInstance::circle(c, 6.0, Paint::solid(hex(GREEN).fade(k))));
            let o = {
                let c = Outline::new(&icons::check(), 0.05);
                c.trim(0.0, (k) * c.length())
            }.transform(
                &Affine::translate(-12.0, -12.0)
                    .then(&Affine::scale(0.36))
                    .then(&Affine::translate(c.x, c.y)),
            );
            p.outline(&o, &StrokeStyle::solid(1.6, hex(0x0b2a18)));
        }
    }
}

impl Popup {
    fn captions_mode(&self) -> Mode {
        self.kit_mode().into()
    }

    pub(crate) fn sync_captions(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Captions {
            return;
        }
        let mode = self.captions_mode();
        let c = &mut self.captions;
        if c.mode != Some(mode) {
            if c.mode.is_some() {
                c.mode_at = t;
            }
            c.mode = Some(mode);
        }
        let f = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if (f - c.prog.1).abs() > 1e-4 {
            let cur = c.prog.0 + (c.prog.1 - c.prog.0) * ease_out((t - c.prog.2) / 0.35);
            c.prog = (cur, f, t);
        }
        self.note_captions_back(t);
    }

    /// The backdrop follows the widest line on screen; a change starts a
    /// new ease from where it is now.
    fn note_captions_back(&mut self, t: f64) {
        let target = self.captions_back_target();
        let (a, b, at) = self.captions.back;
        if (target.w - b.w).abs() > 0.5 || (target.h - b.h).abs() > 0.5 {
            let k = ease_out((t - at) / 0.3);
            self.captions.back = (lerp_rect(a, b, k), target, t);
        }
    }

    fn captions_back_target(&self) -> Rect {
        let style = self.look.text_style();
        let all = self.text_lines(&style, WRAP);
        let shown = &all[all.len().saturating_sub(MAX_LINES as usize)..];
        let widest = shown.iter().map(|l| l.1).fold(0.0f32, f32::max).max(260.0);
        let lines = shown.len().max(1) as f32;
        let w = widest + 110.0;
        let top = SLOTS[0].0 - (lines - 0.5) * LINE - 8.0;
        let bottom = if self.captions.mode == Some(Mode::Finishing)
            || self.captions.mode == Some(Mode::Done)
        {
            STATUS_Y + 18.0
        } else {
            SLOTS[0].0 + LINE / 2.0 + 10.0
        };
        Rect::new((BAND_W - w) / 2.0, top, w, bottom - top)
    }

    pub(crate) fn arm_captions_tick(&mut self, cx: &mut Cx<Msg>) {
        let busy = self.captions.mode == Some(Mode::Finishing) && cx.t() < self.captions.prog.2 + 0.4;
        if self.look.theme == OsdTheme::Captions && self.extra.tick.is_none() && busy && !self.closing
        {
            self.extra.tick = Some(cx.after(1.0 / TICK_HZ, Msg::Tick));
        }
    }

    pub(crate) fn captions_surface(_look: &Look) -> Size {
        Size::new(BAND_W, BAND_H)
    }

    pub(crate) fn captions_view(&self) -> Element<Msg> {
        let size = Self::captions_surface(&self.look);
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        root.child(self.captions_panel(size, 0.0, 0.0))
    }

    /// The screen-edge glow while the mic is live: a fullscreen,
    /// click-through surface under the band. Empty once the mic stops, so
    /// the surface fades out and closes.
    pub(crate) fn captions_surfaces(&self) -> Vec<vo::SurfaceView<Msg>> {
        if self.closing || self.captions.mode != Some(Mode::Listening) {
            return Vec::new();
        }
        // The loudest of the last 0.15 s, so the glow breathes with the
        // syllables rather than flickering with each frame.
        let n = self.wave.len();
        let level = self.wave[n.saturating_sub(15)..]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b))
            .clamp(0.0, 1.0);
        let spec = OverlaySpec {
            // A guess until the compositor gives the output's size.
            size: Size::new(1920.0, 1080.0),
            placement: Placement::Fullscreen,
            input: InputMode::None,
            layer: vo::Stack::Top,
            app_id: "voxtype-osd-edge".into(),
            ..OverlaySpec::default()
        };
        let edge = Element::panel("edge")
            .grow(1.0)
            .stroke(hex(EDGE).fade(0.35 + 0.35 * level), 1.5)
            .glow(hex(EDGE).fade(0.7), 18.0 + 30.0 * level, 0.2 + 0.5 * level)
            .motion(Prop::Opacity, Spring::critical_to_98(0.3));
        let root = Element::panel("edge-root")
            .presence(Transition::fade(), Transition::fade())
            .child(edge);
        vec![vo::SurfaceView::new("edge", spec, root)]
    }

    fn captions_panel(&self, size: Size, x: f32, y: f32) -> Element<Msg> {
        let l = &self.look;
        let c = &self.captions;
        let mode = c.mode.unwrap_or(Mode::Loading);
        let f = fonts(&l.fonts);
        let style = l.text_style();
        // While editing: the window's lines, each with its sticky centring
        // width.
        let all = self.text_lines(&style, WRAP);
        let editing = self.edit_view.is_some();
        // From a tap until the edit is handed back, a line whose first word
        // changes (an edit reflowed it) swaps to its new node at once, so no
        // word shows twice.
        let tapped = self.edit_tapped();
        // With fold-under (MO-33) the line that leaves the top stays in the
        // last slot, turned away about its baseline, instead of fading up.
        let fold = matches!(l.overflow(), Overflow::Fold(_));
        let kept = if fold { SLOTS.len() } else { SLOTS.len() - 1 };
        let lines = &all[all.len().saturating_sub(kept)..];
        let n = lines.len();
        let roll = Spring::critical_to_98(ROLL_SECS);
        let fold_angle = match l.overflow() {
            Overflow::Fold(f) => f.angle,
            _ => 0.0,
        };
        // A row turns about its centre line, so a folding row's box is as
        // tall as twice its baseline.
        let base = self
            .measure
            .layout("x", &style, None)
            .glyphs
            .first()
            .map_or(LINE * 0.7, |g| g.pos.y);
        let live = mode == Mode::Listening && !tapped;
        // The pitch channel lifts the newest word up to the reading line's
        // 3 px; a rise on top of that starts it 5.5 px off its rest (RD-09),
        // so with pitch a word only fades in.
        let pitch = l.on(super::channels::PITCH) || l.on(super::channels::VOICE);
        let enter_word = if pitch {
            Transition::fade()
        } else {
            Transition::fade().offset(0.0, 6.0)
        };
        let newest = if live {
            self.model.words().last().map(|w| w.id)
        } else {
            None
        };

        let mut doubts = Vec::new();
        let mut dot = None;
        let mut newest_line: Option<(Vec<Word>, f32, f32)> = None;
        let mut top_cy = SLOTS[0].0;
        let mut texts = Vec::new();
        let mut sculpt_rows = Vec::new();
        for (i, (words, w)) in lines.iter().enumerate() {
            let age = n - 1 - i;
            let (cy, opacity) = SLOTS[age.min(SLOTS.len() - 1)];
            // Both edited lines are full ink, so either can hold the caret.
            let opacity = if editing { 1.0 } else { opacity };
            let lx = (BAND_W - w) / 2.0;
            let ly = cy - LINE / 2.0;
            let leaving = age + 1 >= SLOTS.len();
            if age == 0 && live {
                dot = Some(Point::new(lx - 22.0, cy + 1.0));
            }
            if age == 0 {
                newest_line = Some((words.clone(), lx, cy));
            }
            if !leaving {
                top_cy = top_cy.min(cy);
            }
            // The newest word in the accent colour: the line is drawn as the
            // words before it and the word itself, side by side.
            let (body, hot) = match (newest, words.last()) {
                (Some(id), Some(last)) if age == 0 && last.id == id => {
                    (&words[..words.len() - 1], Some(last))
                }
                _ => (&words[..], None),
            };
            // While live, a word reaches the body from the hot node, where it
            // already came in, so the body takes it as it stands: a second
            // entry there dimmed it to 79 % ink and dropped it 4 px.
            let line_el = |key: String, ws: &[Word], colour: Color, enter: bool| {
                let text = Element::live_text(key, ws)
                    .text_style(style)
                    .revision_motion(revision::motion(&self.look))
                    .word_revise(revision::revise(&self.look))
                    .color(colour)
                    .motion(Prop::TextColor, Spring::critical_to_98(0.15))
                    .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                    .motion(Prop::Offset, Spring::POP)
                    .word_enter(if tapped || !enter {
                        Transition::NONE
                    } else {
                        self.ghost_enter(enter_word)
                    });
                // MO-14 and the other word channels.
                self.typo_words(text)
            };
            // Each part gets its own measured width, so neither wraps.
            let width = |ws: &[Word]| {
                let joined = ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                self.measure.layout(&joined, &style, None).size.w + 2.0
            };
            let space = self.measure.layout("x x", &style, None).size.w
                - 2.0 * self.measure.layout("x", &style, None).size.w;
            let bw = if body.is_empty() { 0.0 } else { width(body) - 2.0 + space };
            // An edited line is the buffer's own text, spaces and all, so it
            // gets the whole wrap width: its own words' width could wrap it.
            let body_w = if editing { WRAP } else { width(body) };
            let key = words
                .first()
                .map_or_else(|| format!("slot-{i}"), |w| w.id.to_string());
            let mut row = Element::column(format!("line-{key}"))
                .size(*w + 4.0, LINE)
                .child(at(
                    line_el("body".into(), body, hex(WHITE), !live).width(body_w),
                    0.0,
                    0.0,
                ));
            if let Some(hot) = hot {
                let one = std::slice::from_ref(hot);
                row = row.child(at(
                    line_el(format!("hot-{}", hot.id), one, hex(ACCENT), true).width(width(one)),
                    bw,
                    0.0,
                ));
            }
            if age == 0 {
                // The ghost tail after the newest line, as far as the text's
                // right edge: the line is centred, so that is half the free room.
                let tail = model_signals::TailText {
                    style,
                    width: (WRAP + w) / 2.0,
                    rows: 1,
                };
                row = self.signal_marks_in(row, words, &tail, (0.0, 0.0), hex(WHITE));
            }
            if fold {
                row = row
                    .size(*w + 4.0, 2.0 * base)
                    .rotate_x(if leaving { fold_angle } else { 0.0 })
                    .motion(Prop::Tilt, Spring::critical_to_98(FOLD_SECS));
            }
            if !leaving {
                sculpt_rows.push((words.clone(), Point::new(x + lx, y + ly)));
            }
            texts.push(
                at(row, x + lx, y + ly)
                    .opacity(if mode == Mode::Cancelled { opacity * 0.4 } else { opacity })
                    .motion(Prop::Position, roll)
                    .motion(Prop::Opacity, roll)
                    .motion(Prop::Size, Motion::Snap)
                    // A new row comes in through its words' own entry: a fade
                    // and rise of the whole row put the newest words 10 px low
                    // at 4 % ink, past the reading-line envelope (RD-09).
                    // Rows only roll up while listening. After the stop the
                    // final text re-keys every row in place, so the old rows
                    // go at once: rising as they faded, they drew over the
                    // final rows.
                    .presence(
                        Transition::NONE,
                        if tapped || !matches!(mode, Mode::Listening | Mode::Loading) {
                            Transition::NONE
                        } else {
                            Transition::fade().offset(0.0, -18.0)
                        },
                    ),
            );
            // Unsure words on this line get the underline.
            if live && !leaving {
                let mut px = lx;
                for (k, wd) in words.iter().enumerate() {
                    let ww = self.measure.layout(&wd.text, &style, None).size.w;
                    let conf = self.model.info(wd.id).and_then(|i| i.conf);
                    if conf.is_some_and(|c| c < self.look.doubt_below) && newest != Some(wd.id) {
                        doubts.push((px + ww * 0.15, cy + LINE * 0.34, ww * 0.55));
                    }
                    let gap = if k + 1 < words.len() { space } else { 0.0 };
                    px += ww + gap;
                }
            }
        }

        if lines.is_empty() {
            // No line yet: the forming cue in the newest slot, from its middle.
            let tail = model_signals::TailText {
                style,
                width: WRAP / 2.0,
                rows: 1,
            };
            let slot = Element::layer("forming-slot").size(WRAP / 2.0, LINE);
            let slot = self.signal_marks_in(slot, &[], &tail, (0.0, 0.0), hex(WHITE));
            texts.push(at(slot, x + BAND_W / 2.0, y + SLOTS[0].0 - LINE / 2.0));
        }

        let status = self.status();
        let ui = TextStyle::new(f.ui, 14.0, 18.0).tracking(0.1);
        let (label, colour) = match mode {
            Mode::Finishing => (
                format!("{}{}", status.state, status.detail),
                hex(HINT),
            ),
            Mode::Loading => (
                format!("{}{}", status.state, status.detail),
                hex(HINT),
            ),
            Mode::Done => {
                let app = self
                    .files
                    .live
                    .as_ref()
                    .and_then(|d| d.target.as_ref())
                    .map(|t| t.app.clone())
                    .filter(|a| !a.is_empty());
                (
                    app.map_or(self.delivery_word().to_string(), |a| {
                        format!("{} into {a}", self.delivery_word())
                    }),
                    hex(GREEN),
                )
            }
            _ => (String::new(), hex(HINT)),
        };
        let lw = self.measure.layout(&label, &ui, None).size.w;
        let check = (mode == Mode::Done).then(|| Point::new((BAND_W - lw) / 2.0 - 4.0, STATUS_Y + 9.0));
        let label_x = (BAND_W - lw) / 2.0 + if check.is_some() { 8.0 } else { 0.0 };
        let prog = (mode == Mode::Finishing && self.files.fraction.is_some()).then(|| {
            (
                Rect::new((BAND_W - 210.0) / 2.0, STATUS_Y - 9.0, 210.0, 3.0),
                c.prog,
            )
        });

        let scene = Scene {
            mode,
            mode_at: c.mode_at,
            shown_at: self.shown_at,
            back: c.back,
            dot,
            doubts,
            prog,
            check,
            idle: !self.closing
                && matches!(mode, Mode::Loading | Mode::Listening)
                && !reduced()
                && !l.on("reduced-motion"),
            sky: self.captions_sky(newest_line.as_ref(), top_cy, base, &style, mode),
        };
        let until = scene.until(self.extra.now);
        let canvas = at(
            Element::canvas("band", until, move |t, r, p| scene.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h);
        let mut panel = at(Element::layer("captions"), x, y)
            .size(size.w, size.h)
            .presence(Transition::fade(), Transition::fade())
            .child(canvas);
        if l.on(wild_space::SOUND_SCULPTURE) {
            // Letter bodies behind the text, as deep as each word was loud.
            let done = !matches!(mode, Mode::Listening | Mode::Loading) || self.still();
            if let Some(e) = self.sculpture_element(&sculpt_rows, &style, size, roll, done) {
                panel = panel.child(e);
            }
        }
        panel = panel.children(texts);
        panel = panel.children(self.captions_carriage(newest_line.as_ref(), &style, mode));
        let ink = endings::Ink {
            text: hex(WHITE),
            mark: hex(ACCENT),
            faint: hex(HINT),
        };
        panel = panel.children(self.unthread_rows(&sculpt_rows, style, size, ink));
        if !label.is_empty() {
            panel = panel.child(
                at(
                    // Keyed once, not by state: a new key per state faded
                    // the old word out under the new one ("Typed" over "Done").
                    Element::label("status", label).text_style(ui).color(colour),
                    x + label_x,
                    y + STATUS_Y,
                )
                .presence(Transition::fade().offset(0.0, 4.0), Transition::fade()),
            );
        }
        panel
    }
}

impl Popup {
    /// R2-13: each word of a line that starts at `lx`, as its x span.
    fn captions_spans(&self, words: &[Word], lx: f32, style: &TextStyle) -> Vec<(f32, f32)> {
        let width = |t: &str| self.measure.layout(t, style, None).size.w;
        let space = width("x x") - 2.0 * width("x");
        let mut x = lx;
        words
            .iter()
            .map(|w| {
                let ww = width(&w.text);
                let span = (x, x + ww);
                x += ww + space;
                span
            })
            .collect()
    }

    /// R2-13-a and R2-13-b: the strip above the top line, from the band's
    /// top to just over the top line's letters, with the rings and the fan
    /// that are on. `base` is a line's baseline under its top.
    fn captions_sky(
        &self,
        newest: Option<&(Vec<Word>, f32, f32)>,
        top_cy: f32,
        base: f32,
        style: &TextStyle,
        mode: Mode,
    ) -> Option<Sky> {
        let l = &self.look;
        let (echo, prism) = (l.on(LIQUID_ECHO), l.on(PRISM_CAPTURE));
        if !(echo || prism)
            || !matches!(mode, Mode::Listening | Mode::Loading)
            || self.edit_view.is_some()
        {
            return None;
        }
        let still = self.still() || reduced();
        let back = self.captions_back_target();
        let bottom = top_cy - LINE / 2.0 + base - 0.8 * style.size - 3.0;
        // At most `SKY_H` tall, just over the text.
        let top = (bottom - SKY_H).max(4.0);
        let strip = Rect::new(
            back.x + back.h / 2.0,
            top,
            (back.w - back.h).max(40.0),
            (bottom - top).max(0.0),
        );
        if strip.h < 6.0 {
            return None;
        }
        let rings = echo.then(|| {
            let now = self.extra.now;
            newest.map_or_else(Vec::new, |(words, lx, _)| {
                words
                    .iter()
                    .zip(self.captions_spans(words, *lx, style))
                    .filter_map(|(w, (x0, x1))| {
                        let born = self.heard.word(w.id)?.born;
                        (0.0..ECHO_SECS)
                            .contains(&(now - born))
                            .then_some(((x0 + x1) / 2.0, born))
                    })
                    .collect()
            })
        });
        let n = self.wave.len();
        let level = self.wave[n.saturating_sub(15)..]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b))
            .clamp(0.0, 1.0);
        Some(Sky {
            strip,
            rings,
            prism: prism.then_some(level),
            still,
        })
    }

    /// R2-13-c: the ruler under the newest line, its carriage marker at the
    /// newest word's end and, in a pause, a box round the newest phrase.
    /// While listening only.
    fn captions_carriage(
        &self,
        newest: Option<&(Vec<Word>, f32, f32)>,
        style: &TextStyle,
        mode: Mode,
    ) -> Vec<Element<Msg>> {
        if !self.look.on(TYPEWRITER_SET) || mode != Mode::Listening || self.edit_view.is_some() {
            return Vec::new();
        }
        let Some((words, lx, cy)) = newest.filter(|n| !n.0.is_empty()) else {
            return Vec::new();
        };
        let still = self.still() || reduced();
        let spans = self.captions_spans(words, *lx, style);
        let back = self.captions_back_target();
        let y = cy + LINE / 2.0 + 9.0;
        let (x0, x1) = (back.x + back.h / 2.0, back.x + back.w - back.h / 2.0);
        let ruler = Element::canvas("ruler", 0.0, move |_, r, p| {
            let n = ((x1 - x0) / RULER_TICK) as usize;
            p.shape(ShapeInstance::line(
                Point::new(r.x, r.y + 4.0),
                Point::new(r.x + r.w, r.y + 4.0),
                1.0,
                Paint::solid(Color::WHITE.fade(0.14)),
            ));
            for i in 0..=n {
                let x = r.x + i as f32 * RULER_TICK;
                let h = if i % 5 == 0 { 5.0 } else { 2.5 };
                p.shape(ShapeInstance::line(
                    Point::new(x, r.y + 4.0 - h),
                    Point::new(x, r.y + 4.0),
                    1.0,
                    Paint::solid(Color::WHITE.fade(0.18)),
                ));
            }
        });
        let end = spans.last().map_or(*lx, |s| s.1);
        let glide = if still {
            Motion::Snap
        } else {
            Spring::critical_to_98(CARRIAGE_SECS).into()
        };
        let marker = Element::panel("carriage")
            .size(4.0, 11.0)
            .radius(2.0)
            .fill(hex(ACCENT))
            .glow(hex(ACCENT).fade(0.6), 5.0, 0.5);
        let mut out = vec![
            at(ruler.size(x1 - x0, 8.0), x0, y).presence(Transition::fade(), Transition::fade()),
            at(marker, end + 2.0, y - 3.5)
                .motion(Prop::Position, glide)
                .presence(Transition::fade(), Transition::fade()),
        ];
        // The phrase: the newest words, each within `PHRASE_GAP` of the one
        // before it; framed once no word has come for `PHRASE_PAUSE`.
        let births: Vec<f64> = words
            .iter()
            .filter_map(|w| self.heard.word(w.id).map(|h| h.born))
            .collect();
        if births.len() == words.len() && self.extra.now - births[births.len() - 1] >= PHRASE_PAUSE
        {
            let mut first = births.len() - 1;
            while first > 0 && births[first] - births[first - 1] <= PHRASE_GAP {
                first -= 1;
            }
            let (px0, px1) = (spans[first].0 - 6.0, spans[spans.len() - 1].1 + 6.0);
            let frame = Element::panel(format!("phrase-{}", words[first].id))
                .size(px1 - px0, LINE - 6.0)
                .radius(3.0)
                .stroke(hex(ACCENT).fade(0.55), 1.0);
            out.push(
                at(frame, px0, cy - LINE / 2.0 + 3.0)
                    .presence(Transition::fade(), Transition::fade()),
            );
        }
        out
    }
}

/// Captions' edit adapter (plan section 4.5): one node per line, the hot
/// word merged into its line, the default caret.
pub(crate) struct CaptionsSite;

impl edit_site::EditSite for CaptionsSite {
    fn place(&self, look: &Look, el: Element<Msg>, view: &edit::EditView) -> Element<Msg> {
        edit_site::place_lines(self, look, el, view)
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::scope::edit_tests::check_edit_envelope;
    use super::super::swiss_look::edit_check::Rig;
    use super::*;

    /// M7.L8: Captions edits in its own band. With the caret on the newest
    /// and the top line, at both widths: the caret on its glyph within 1 px
    /// in every frame, the popup's size unchanged, and the golden
    /// `tests/fixtures/osd-vo/edit-captions.txt`.
    #[test]
    fn captions_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Captions, "captions");
    }

    /// While editing, a line is one live text with the newest word in it,
    /// and both lines are at full ink.
    #[test]
    fn captions_edit_lines_are_whole_and_full_ink() {
        let mut r = Rig::new(OsdTheme::Captions, false);
        r.dictate(40);
        r.enter(1.0);
        let view = r.p.view(&vo::Tokens::default());
        let mut keys = Vec::new();
        let _ = view.map_live_texts(&mut |e| {
            keys.push(e.key().to_string());
            e
        });
        assert!(!keys.iter().any(|k| k.starts_with("hot-")), "{keys:?}");
        let n = r.p.edit_view.as_ref().unwrap().lines;
        assert_eq!(n, MAX_LINES);
        assert_eq!(keys.iter().filter(|k| *k == "body").count(), n as usize);
    }

    /// A centred line keeps its left edge while you type, and re-centres
    /// 0.5 s after the last key.
    #[test]
    fn captions_lines_hold_still_while_typing() {
        use vo_core::edit::EditAction;
        let mut r = Rig::new(OsdTheme::Captions, false);
        // A short newest line, so typing does not run it past the box.
        r.dictate(30);
        r.enter(1.0);
        let left = |r: &Rig| {
            let v = r.p.edit_view.as_ref().unwrap();
            v.centred_left(v.caret_line as usize, BAND_W)
        };
        let before = left(&r);
        r.act(EditAction::Insert("abc".into()), 2.0);
        assert_eq!(left(&r), before, "while typing");
        r.act(EditAction::Insert("d".into()), 2.2);
        assert_eq!(left(&r), before, "while typing");
        r.show(2.8);
        assert!(r.p.edit_text(2.8));
        assert!(left(&r) < before, "re-centred: {} then {before}", left(&r));
    }

    /// R35: the edit moment with the caret on the newest and the top line
    /// stays inside the reading-line envelope, at both widths.
    #[test]
    fn captions_edit_moment_keeps_the_envelope() {
        check_edit_envelope(OsdTheme::Captions, &[0, 1]);
    }

    /// The hot word keeps its ink when the next word comes and it moves to
    /// the line's body: it came in once, as the hot word.
    #[test]
    fn captions_hot_word_keeps_its_ink_in_the_body() {
        use super::super::moments::{scenario, Moment};
        use vo::scene::display::ItemKind;
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let cfg = OsdConfig {
            theme: OsdTheme::Captions,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        look.set_motion(&cfg.motion);
        let options = Options {
            fonts: fonts.clone(),
            default_motion: default_motion(&look),
            ..Options::default()
        };
        // Word 2 comes at 1.19 s and moves to the body when word 3 comes at
        // 1.51 s.
        let mut low = Vec::new();
        vo::envelope::sample_scenario(
            Popup::new(look.clone(), &fonts),
            &scenario(&look, Moment::Cancel),
            30.0,
            options,
            &mut |_, t, dl| {
                if !(1.4..2.0).contains(&t) {
                    return;
                }
                for (i, _) in dl.layers.iter().enumerate() {
                    for it in dl
                        .layer_items(i as _)
                        .iter()
                        .filter(|it| it.kind == ItemKind::Glyphs)
                    {
                        let glyphs = &dl.glyphs[it.start as usize..(it.start + it.len) as usize];
                        for g in glyphs.iter().filter(|g| g.word == 2 && g.color.a < 0.99) {
                            low.push(format!("{t:.3}: {:.2}", g.color.a));
                        }
                    }
                }
            },
        );
        assert!(low.is_empty(), "word 2 below full ink: {low:?}");
    }

    /// RD-09 with the pitch channel on, as in a real dictation: a word that
    /// comes in while its pitch lifts it stays inside the reading-line
    /// envelope, at both widths. Words came in 5.5 px low when the entry
    /// rose on top of the pitch.
    #[test]
    fn captions_pitched_words_keep_the_envelope() {
        use super::super::moments::{scenario, Moment};
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        for wide in [false, true] {
            let mut cfg = OsdConfig {
                theme: OsdTheme::Captions,
                ..OsdConfig::default()
            };
            if wide {
                cfg.width_px = 600;
                cfg.preview_font_pt = Some(17.0);
            }
            let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
            look.set_motion(&cfg.motion);
            assert!(look.on(super::super::channels::PITCH));
            let options = Options {
                fonts: fonts.clone(),
                default_motion: default_motion(&look),
                ..Options::default()
            };
            let report = vo::envelope::check_scenario(
                Popup::new(look.clone(), &fonts),
                &scenario(&look, Moment::Cancel),
                30.0,
                options,
                &vo::envelope::Envelope {
                    transcript: 0..1 << 20,
                    ..vo::envelope::Envelope::default()
                },
            );
            assert!(report.is_ok(), "wide {wide}: {report}");
        }
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn the_bars_stay_out_of_the_newest_line_and_inside_the_backdrop() {
        let b = Rect::new(100.0, 19.0, 900.0, 126.0);
        let [top, bottom] = bars(b);
        assert!(top.y + top.h <= SLOTS[0].0 - LINE / 2.0);
        assert!(bottom.y >= SLOTS[0].0 + LINE / 2.0);
        for r in [top, bottom] {
            assert!(r.x >= b.x + b.h / 2.0 && r.x + r.w <= b.x + b.w - b.h / 2.0 + 0.01);
            assert!(r.y >= b.y && r.y + r.h <= b.y + b.h + 0.01);
        }
    }

    // The grain and the flicker stay faint and at most 12 steps a second.
    const _: () = assert!(GRAIN_ALPHA <= 0.03 && FLICKER <= 0.02 && IDLE_FPS <= 12.0);

    #[test]
    fn the_sky_draws_until_its_last_ring_or_while_the_fan_follows_the_voice() {
        let strip = Rect::new(0.0, 10.0, 600.0, 30.0);
        let rings = Sky {
            strip,
            rings: Some(vec![(100.0, 2.0), (200.0, 2.5)]),
            prism: None,
            still: false,
        };
        assert_eq!(rings.until(2.6), 2.5 + ECHO_SECS);
        let fan = Sky {
            strip,
            rings: None,
            prism: Some(0.5),
            still: false,
        };
        assert_eq!(fan.until(3.0), 3.25);
        // Within the subtle limits of the bead.
        const { assert!(ECHO_ALPHA <= 0.2 && FAN_ALPHA <= 0.25 && SKY_H <= 40.0) };
    }

    #[test]
    fn the_flicker_changes_from_step_to_step() {
        let mut seen = std::collections::HashSet::new();
        for n in 0..24u32 {
            let v = noise(n.wrapping_mul(31));
            assert!((0.0..=1.0).contains(&v));
            seen.insert((v * 100.0) as u32);
        }
        assert!(seen.len() > 8);
    }
}
