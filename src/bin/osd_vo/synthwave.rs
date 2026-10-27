//! The `synthwave` look of the vo popup: a dark card inside a magenta to
//! cyan neon tube, a chrome italic title that names the state, a neon bar
//! waveform over a perspective grid, and bright white centred words.
//!
//! The tube draws itself on from the bottom middle when the popup opens and
//! then flickers on like a real neon tube; when a dictation is cancelled it
//! powers down from the right. The title's letters track in from wide
//! spacing whenever the state changes. The grid scrolls toward the viewer
//! while the recording runs and coasts to a stop after it. Once the final
//! text lands, a ring and a check draw on where the waveform was.
//!
//! The tube, card, grid and waveform are canvases, pure in the frame time,
//! so they move without view updates.

use super::*;

use std::sync::OnceLock;

use vo::scene::display::{BlendMode, Paint, ShapeInstance};
use vo::scene::{Loop, LoopProp};
use vo::scene::geom::{Point, RRect};
use vo::scene::path::{icons, make, Contour, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo::scene::taffy::JustifyContent;
use vo::scene::text::TextLayout;

/// Layout at the design width of 600 logical pixels; everything scales
/// with the popup's width.
const DESIGN_W: f32 = 600.0;
/// Transparent room round the card for the tube's glow.
const MARGIN: f32 = 18.0;
/// Card height above the first transcript line and below the last one.
const HEAD: f32 = 233.0;
const FOOT: f32 = 73.0;
/// Transcript size and line pitch.
const TEXT_PX: f32 = 33.0;
const TEXT_LINE: f32 = 42.0;
/// Transcript width.
const TEXT_W: f32 = 520.0;
/// Title size and centre line.
const TITLE_PX: f32 = 72.0;
const TITLE_Y: f32 = 55.0;
/// Waveform centre line, tallest half height and bar pitch.
const WAVE_Y: f32 = 151.0;
const WAVE_HALF: f32 = 62.0;
const WAVE_X0: f32 = 18.0;
const WAVE_X1: f32 = 582.0;
const BAR_PITCH: f32 = 7.1;
const BAR_W: f32 = 2.8;
/// Bottom of the grid under the waveform.
const GRID_BOTTOM: f32 = 262.0;
/// Status row centre, above the card's bottom edge.
const STATUS_UP: f32 = 40.0;
const STATUS_PX: f32 = 18.5;
/// The tube: stroke width and the corner cuts.
const TUBE_W: f32 = 2.4;
const CUT: f32 = 30.0;

/// Seconds for the tube to draw on, then to flicker on.
const DRAW_SECS: f64 = 0.52;
const FLICKER_SECS: f64 = 0.30;
/// Seconds for the card to follow a new line.
const GROW_SECS: f64 = 0.32;
/// Seconds for the waveform to settle to dots after the stop.
const SETTLE_SECS: f64 = 0.4;
/// Seconds for the grid to coast to a stop, and per grid row while running.
const COAST_SECS: f64 = 0.6;
const GRID_ROW_SECS: f64 = 1.2;
/// Seconds for the tube to power down on a cancel.
const POWER_DOWN_SECS: f64 = 0.45;
/// Seconds the daemon must stay idle without a final pass before the look
/// calls it a cancel, so a stop whose status comes late does not flash one.
const CANCEL_CONFIRM: f64 = 0.08;
/// How far the lit tube's brightness wanders while it hums.
const HUM: f32 = 0.03;
/// Seconds per cycle of the "..." after "Loading".
const DOTS_SECS: f64 = 1.2;
/// When the title first tracks in, after the tube has started drawing.
const TITLE_DELAY: f64 = 0.22;

const MAGENTA: u32 = 0xff2bd6;
const VIOLET: u32 = 0x9b5cff;
const CYAN: u32 = 0x19c8ff;
const BLUE: u32 = 0x3d9bff;
const WHITE: u32 = 0xf6f2ff;

fn hex(rgb: u32) -> Color {
    Color::hex(rgb, 1.0)
}

fn lerp(a: Color, b: Color, k: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

/// The neon hue at `u` across the card, 0 left to 1 right: magenta over
/// the left third, a short violet middle, cyan over the right third.
fn neon(u: f32) -> Color {
    const STOPS: [(f32, u32); 5] = [
        (0.0, MAGENTA),
        (0.36, 0xe030ff),
        (0.5, VIOLET),
        (0.64, 0x4a8cff),
        (1.0, CYAN),
    ];
    let u = u.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let ((u0, c0), (u1, c1)) = (pair[0], pair[1]);
        if u <= u1 {
            return lerp(hex(c0), hex(c1), (u - u0) / (u1 - u0));
        }
    }
    hex(CYAN)
}

/// A fixed pseudo-random value in 0..1 for slot `n`.
fn spike(n: i64) -> f32 {
    let mut x = (n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 32;
    (x & 0xffff) as f32 / 65535.0
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
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

/// The look's own fonts, added to the shared registry once.
struct Fonts {
    title: u32,
    text: u32,
    status: u32,
}

fn fonts(reg: &FontRegistry) -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let add = |pattern: &str, fallback: u32| reg.add_system(pattern).unwrap_or(fallback);
        Fonts {
            title: add("Exo 2:black:italic", SANS),
            text: add("Roboto:medium", SANS),
            status: add("Roboto", SANS),
        }
    })
}

/// The transcript's style, for the line fitting in `Popup::text` too.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let s = look.width / DESIGN_W;
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX * s, TEXT_LINE * s)
}

/// The centre of the newest transcript line, from the surface top. The card
/// grows upward as lines are added, so the newest line stays at this spot.
pub(crate) fn newest_line_centre(look: &Look) -> f32 {
    let s = look.width / DESIGN_W;
    let tall = (HEAD + FOOT + swiss::MAX_LINES as f32 * TEXT_LINE) * s;
    MARGIN + tall - (FOOT + TEXT_LINE / 2.0) * s
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * look.width / DESIGN_W
}

/// What only the synthwave look keeps: when the card last changed height
/// and when a dictation was cancelled.
#[derive(Default)]
pub struct State {
    lines_from: u32,
    lines_to: u32,
    lines_at: f64,
    /// When the daemon went idle with no final pass running.
    ended_at: Option<f64>,
}

/// The card outline as two halves from the bottom middle, left and right
/// round to the top middle, so it draws on both ways at once. `w` and `h`
/// are the card's size; the stroke sits `inset` inside it.
fn tube_halves(w: f32, h: f32, s: f32, inset: f32) -> [Contour; 2] {
    let left = tube_points(w, h, s, inset);
    let right: Vec<Point> = left.iter().map(|p| Point::new(w - p.x, p.y)).collect();
    [
        Contour::polyline(&left, false),
        Contour::polyline(&right, false),
    ]
}

/// The left half of the outline, from the bottom middle to the top middle.
fn tube_points(w: f32, h: f32, s: f32, inset: f32) -> Vec<Point> {
    let (c, i) = (CUT * s, inset);
    let low = 38.0 * s; // bottom corner cut, up the side
    let tab = 11.0 * s; // the lower middle tab's depth
    let foot = 30.0 * s; // the bottom corner cut, along the bottom
    let tab_x = 120.0 * s;
    vec![
        Point::new(w / 2.0, h - i),
        Point::new(tab_x + tab, h - i),
        Point::new(tab_x, h - tab - i),
        Point::new(foot, h - tab - i),
        Point::new(i, h - low),
        Point::new(i, c),
        Point::new(c, i),
        Point::new(w / 2.0, i),
    ]
}

/// Short lines inside the four corners, parallel to the cuts.
fn corner_ticks(w: f32, h: f32, s: f32) -> Vec<Contour> {
    let (c, d) = (CUT * s, 9.0 * s);
    let low = 38.0 * s;
    let foot = 30.0 * s;
    let tab = 11.0 * s;
    let left = vec![
        // Top left: along the cut, with a short drop at its lower end.
        vec![
            Point::new(d, c + d + 10.0 * s),
            Point::new(d, c + 4.0 * s),
            Point::new(c + 4.0 * s, d),
            Point::new(c + 26.0 * s, d),
        ],
        // Bottom left: along the cut and the bottom.
        vec![
            Point::new(d, h - low - 16.0 * s),
            Point::new(d, h - low + 4.0 * s),
            Point::new(foot + 6.0 * s, h - tab - d),
            Point::new(foot + 48.0 * s, h - tab - d),
        ],
    ];
    let mut out = Vec::new();
    for pts in left {
        out.push(Contour::polyline(&pts, false));
        let mirrored: Vec<Point> = pts.iter().map(|p| Point::new(w - p.x, p.y)).collect();
        out.push(Contour::polyline(&mirrored, false));
    }
    out
}

/// The card's fill shape.
fn card_shape(w: f32, h: f32, s: f32) -> Contour {
    card_shape_inset(w, h, s, 0.0)
}

/// The card's outline `inset` inside its edge, as one closed contour.
fn card_shape_inset(w: f32, h: f32, s: f32, inset: f32) -> Contour {
    // Bottom middle up the left to the top middle, then down the right.
    let left = tube_points(w, h, s, inset);
    let mut pts = left[..left.len() - 1].to_vec();
    pts.extend(left.iter().rev().map(|p| Point::new(w - p.x, p.y)));
    // `polyline` draws no closing segment, so the last point is the first.
    Contour::polyline(&pts, true)
}

/// Brightness of the tube `k` seconds after it started to flicker on.
fn flicker_on(k: f64) -> f32 {
    const STEPS: [f32; 5] = [1.0, 0.2, 0.9, 0.35, 1.0];
    if k < 0.0 {
        return 1.0;
    }
    let i = (k / (FLICKER_SECS / STEPS.len() as f64)) as usize;
    STEPS.get(i).copied().unwrap_or(1.0)
}

impl Popup {
    /// Brings the look's own state up to date after a message.
    pub(crate) fn sync_synthwave(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Synthwave {
            return;
        }
        let cancelled = self.ended() && self.extra.finish_at.is_none();
        let st = &mut self.synth;
        if st.lines_to == 0 {
            st.lines_from = self.lines;
            st.lines_to = self.lines;
        } else if self.lines != st.lines_to {
            st.lines_from = st.lines_to;
            st.lines_to = self.lines;
            st.lines_at = t;
        }
        if cancelled && st.ended_at.is_none() {
            st.ended_at = Some(t);
        }
        // The daemon can go idle a moment before it writes the finishing
        // status: that was a stop, not a cancel.
        if !cancelled {
            st.ended_at = None;
        }
    }

    fn scale(&self) -> f32 {
        self.look.width / DESIGN_W
    }

    fn card_h(&self, lines: u32) -> f32 {
        (HEAD + FOOT + lines as f32 * TEXT_LINE) * self.scale()
    }

    pub(crate) fn synthwave_surface(look: &Look) -> Size {
        let s = look.width / DESIGN_W;
        let h = (HEAD + FOOT + swiss::MAX_LINES as f32 * TEXT_LINE) * s;
        Size::new(
            (look.width + 2.0 * MARGIN).ceil(),
            (h + 2.0 * MARGIN).ceil(),
        )
    }

    fn now(&self) -> f64 {
        self.extra.now
    }

    /// When the dictation counts as cancelled: once the daemon has stayed
    /// idle, with no final pass, for `CANCEL_CONFIRM` seconds.
    fn cancel_at(&self) -> Option<f64> {
        let at = self.synth.ended_at? + CANCEL_CONFIRM;
        (self.now() >= at).then_some(at)
    }

    fn cancelled(&self) -> bool {
        self.cancel_at().is_some()
    }

    /// The word the title shows.
    fn title_word(&self) -> &'static str {
        let status = self.status();
        if self.cancelled() {
            "CANCELLED"
        } else if self.extra.final_at.is_some() {
            "PASTED"
        } else if status.loading {
            "LOADING"
        } else if self.extra.finish_at.is_some() || (status.finishing && !self.ended()) {
            // While a stop might still turn out a cancel, the title stays.
            "FINISHING"
        } else {
            "LISTENING"
        }
    }

    pub(crate) fn synthwave_view(&self) -> Element<Msg> {
        let l = &self.look;
        let surface = Self::synthwave_surface(l);
        let root = Element::column("root").size(surface.w, surface.h);
        if self.closing {
            return root;
        }
        let w = l.width;
        let h = self.card_h(self.lines);
        let card = Element::layer("card")
            .size(w, self.card_h(swiss::MAX_LINES as u32))
            .child(at(self.sw_card(), 0.0, 0.0))
            .child(at(self.sw_grid_and_wave(), 0.0, 0.0))
            .child(self.sw_done_mark())
            .child(self.sw_title())
            .child(self.sw_text())
            .child(self.sw_status(h))
            .child(at(self.sw_tube(), -MARGIN, -MARGIN));
        // The card's bottom stays put and it grows upward: its top springs
        // up as lines are added, on the same spring as its drawn height.
        let tall = self.card_h(swiss::MAX_LINES as u32);
        root.child(
            at(card, MARGIN, MARGIN + tall - h)
                .motion(Prop::Position, Spring::critical_to_98(GROW_SECS)),
        )
    }

    /// The card's height at time `t`: it springs to each new line count.
    fn height_fn(&self) -> impl Fn(f64) -> f32 + Send + Sync + 'static {
        let st = &self.synth;
        let (from, to) = (self.card_h(st.lines_from), self.card_h(st.lines_to.max(1)));
        let since = st.lines_at;
        let spring = Spring::critical_to_98(GROW_SECS);
        move |t| {
            if from == to || t <= since {
                return if t <= since { from } else { to };
            }
            to + spring.eval((from - to) as f64, 0.0, t - since).0 as f32
        }
    }

    fn sw_until(&self) -> f64 {
        let mut until = self.shown_at + DRAW_SECS + FLICKER_SECS + 0.1;
        until = until.max(self.synth.lines_at + GROW_SECS + 0.1);
        if let Some(c) = self.cancel_at() {
            until = until.max(c + POWER_DOWN_SECS + 0.1);
        }
        if let Some(f) = self.extra.final_at {
            until = until.max(f + 0.6);
        }
        until
    }

    /// The dark card behind everything.
    fn sw_card(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let height = self.height_fn();
        let shown = self.shown_at;
        let tall = self.card_h(swiss::MAX_LINES as u32);
        Element::canvas("card-fill", self.sw_until(), move |t, rect, p| {
            let h = height(t);
            let open = smoothstep(0.0, 0.25, (t - shown) as f32);
            let shape = card_shape(w, h, s).transform(&vo::scene::geom::Affine::translate(
                rect.x, rect.y,
            ));
            let style = StrokeStyle::solid(0.0, Color::hex(0x0d0828, 0.9 * open)).linear(
                Point::new(rect.x, rect.y),
                Color::hex(0x0a0722, 0.92 * open),
                Point::new(rect.x, rect.y + h),
                Color::hex(0x050314, 0.94 * open),
            );
            p.fill([&shape], &style);
        })
        .size(w, tall)
    }

    /// The neon tube and the corner ticks, with the glow and bloom.
    fn sw_tube(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let height = self.height_fn();
        let shown = self.shown_at;
        let cancel = self.cancel_at();
        let pulse = self.extra.final_at;
        let surface = Self::synthwave_surface(&self.look);
        let m = MARGIN;
        let draw = Element::canvas("tube", self.sw_until(), move |t, rect, p| {
            let h = height(t);
            let (ox, oy) = (rect.x + m, rect.y + m);
            let move_to = vo::scene::geom::Affine::translate(ox, oy);
            let k = ((t - shown) / DRAW_SECS).clamp(0.0, 1.0);
            let drawn = 1.0 - (1.0 - k as f32).powi(3);
            let mut bright = flicker_on(t - shown - DRAW_SECS);
            if let Some(f) = pulse {
                let a = ((t - f) / 0.4).clamp(0.0, 1.0) as f32;
                if t >= f {
                    bright *= 1.0 + 0.5 * (1.0 - a);
                }
            }
            // A cancel powers the tube down from the right.
            let down = cancel.map_or(0.0, |c| ((t - c) / POWER_DOWN_SECS).clamp(0.0, 1.0) as f32);
            let gradient = |a: f32, width: f32, glow: f32| {
                StrokeStyle::solid(width, hex(MAGENTA))
                    .linear(
                        Point::new(ox, oy),
                        hex(MAGENTA).fade(a),
                        Point::new(ox + w, oy),
                        hex(CYAN).fade(a * (1.0 - 0.8 * down)),
                    )
                    .glow(glow)
            };
            let fade = 1.0 - 0.75 * down;
            let width = TUBE_W * s.max(1.0);
            if k >= 1.0 {
                // Drawn on: one closed line, so no caps meet at the seams.
                let ring = card_shape_inset(w, h, s, TUBE_W / 2.0).transform(&move_to);
                p.stroke(&ring, &gradient(bright * fade, width, 6.0));
                p.stroke(
                    &ring,
                    &StrokeStyle::solid(0.9, Color::hex(0xffffff, 0.55 * bright * fade)),
                );
            }
            for half in tube_halves(w, h, s, TUBE_W / 2.0).into_iter().filter(|_| k < 1.0) {
                let half = half.transform(&move_to);
                let len = half.length() * drawn;
                let piece = half.trim(0.0, len);
                p.stroke(&piece, &gradient(bright * fade, width, 6.0));
                // A white hot core along the tube.
                p.stroke(
                    &piece,
                    &StrokeStyle::solid(0.9, Color::hex(0xffffff, 0.55 * bright * fade)),
                );
                // The hot tip while drawing on.
                if k < 1.0 {
                    if let Some((tip, _)) = half.at(len) {
                        p.shape(ShapeInstance::circle(
                            tip,
                            2.6,
                            Paint::solid(Color::hex(0xffffff, 0.95)),
                        ));
                    }
                }
            }
            // The corner ticks light once the tube has passed them.
            let ticks = smoothstep(0.6, 1.0, drawn) * bright * fade;
            for c in corner_ticks(w, h, s) {
                let c = c.transform(&move_to);
                let len = c.length();
                p.stroke(&c.trim(0.0, len * ticks.min(1.0)), &gradient(0.75 * ticks, 1.1, 2.0));
            }
        })
        .size(surface.w, surface.h);
        let layer = Element::layer("tube-light")
            .size(surface.w, surface.h)
            .blend(BlendMode::Add)
            .bloom(0.9, 8.0)
            .opacity(1.0 - HUM)
            .child(draw);
        // Once the tube is on and until a cancel, it hums: its brightness and
        // its bloom wander a little, as a real neon tube's do.
        let lit = self.now() >= self.shown_at + DRAW_SECS + FLICKER_SECS;
        if lit && !self.cancelled() {
            layer
                .ambient(LoopProp::Opacity, Loop::drift(HUM as f64, 0.11, 7))
                .ambient(LoopProp::BloomStrength, Loop::drift(0.12, 0.3, 11))
        } else {
            layer
        }
    }

    /// The perspective grid and the bar waveform, in one bloom layer.
    fn sw_grid_and_wave(&self) -> Element<Msg> {
        let l = &self.look;
        let (w, s) = (l.width, self.scale());
        let shown = self.shown_at;
        let status = self.status();
        let stop = self.extra.finish_at.or(self.cancel_at());
        let done = self.extra.final_at;
        let loading = status.loading;
        let fraction = self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32);
        let finishing = self.extra.finish_at.is_some() && done.is_none();
        let horizon = WAVE_Y * s;
        let bottom = GRID_BOTTOM * s;
        let grid_until = stop.map_or(f64::INFINITY, |f| f + COAST_SECS + 0.05);
        let grid = Element::canvas("grid", grid_until, move |t, rect, p| {
            // Rows scroll toward the viewer and coast to a stop after the stop.
            let run = match stop {
                Some(f) if t > f => {
                    let k = ((t - f) / COAST_SECS).min(1.0);
                    (f - shown) + COAST_SECS * (k - 0.5 * k * k)
                }
                _ => t - shown,
            }
            .max(0.0);
            let phase = (run / GRID_ROW_SECS).fract() as f32;
            let open = smoothstep(0.1, 0.5, (t - shown) as f32);
            let (x0, y0) = (rect.x, rect.y + horizon);
            let span = bottom - horizon;
            let cx = x0 + w / 2.0;
            // Rays from a vanishing point a little above the horizon.
            let vy = y0 - 10.0 * s;
            for k in -10..=10 {
                let bx = cx + k as f32 * 64.0 * s;
                let along = |y: f32| cx + (bx - cx) * (y - vy) / (bottom + rect.y - vy);
                let (a, b) = (Point::new(along(y0), y0), Point::new(along(y0 + span), y0 + span));
                let hue = neon((bx - x0) / w);
                let style = StrokeStyle::solid(1.0, hue).linear(
                    a,
                    hue.fade(0.55 * open),
                    b,
                    hue.fade(0.0),
                );
                p.stroke(&Contour::polyline(&[a, b], false), &style);
            }
            // Rows at depth z land at y = horizon + span / z.
            for n in 0..10 {
                let z = 1.0 + (n as f32 - phase) * 0.7;
                if z < 0.8 {
                    continue;
                }
                let y = y0 + span / z * 0.92;
                if y > y0 + span {
                    continue;
                }
                let depth = (y - y0) / span;
                let a = open * 0.55 * (1.0 - depth).powf(0.8) * smoothstep(0.02, 0.12, depth);
                let style = StrokeStyle::solid(1.0, hex(VIOLET)).linear(
                    Point::new(x0, y),
                    hex(MAGENTA).fade(a),
                    Point::new(x0 + w, y),
                    hex(CYAN).fade(a),
                );
                p.stroke(
                    &Contour::polyline(&[Point::new(x0, y), Point::new(x0 + w, y)], false),
                    &style,
                );
            }
        })
        .size(w, bottom + 4.0);
        let grid = Element::layer("grid-fade")
            .size(w, bottom + 4.0)
            .fade_edges(110.0 * s, 0.0, 110.0 * s, 40.0 * s)
            .child(grid);

        let levels = self.wave.clone();
        let newest_at = self.newest_at;
        let window = l.window;
        let (wx0, wx1) = (WAVE_X0 * s, WAVE_X1 * s);
        let half_max = WAVE_HALF * s;
        let pitch = BAR_PITCH * s;
        let bar_w = BAR_W * s;
        let wave_until = match (stop, done) {
            (_, Some(d)) => d + 0.4,
            (Some(f), None) if !finishing => f + SETTLE_SECS + 0.05,
            (Some(_), None) => f64::INFINITY,
            (None, None) => f64::INFINITY,
        };
        let wave = Element::canvas("wave", wave_until, move |t, rect, p| {
            let open = smoothstep(0.15, 0.5, (t - shown) as f32);
            let settle = stop.map_or(1.0, |f| {
                1.0 - smoothstep(0.0, 1.0, ((t - f) / SETTLE_SECS) as f32)
            });
            let gone = done.map_or(1.0, |d| 1.0 - smoothstep(0.0, 1.0, ((t - d) / 0.3) as f32));
            let cy = rect.y + horizon;
            let n = ((wx1 - wx0) / pitch) as i32;
            let now = t.min(newest_at.max(shown));
            let slot = window / n as f64;
            // Bars sit on a grid of slot times, so a bar keeps its height while it scrolls.
            let last = (now / slot).floor() as i64;
            let shift = ((now / slot).fract()) as f32 * pitch;
            for j in 0..=n {
                let x = rect.x + wx1 - j as f32 * pitch - shift;
                if x < rect.x + wx0 {
                    break;
                }
                let u = (x - rect.x - wx0) / (wx1 - wx0);
                let slot_t = (last - j as i64) as f64 * slot;
                let age = newest_at - slot_t;
                let i = levels.len() as f64 - 1.0 - age * FRAME_HZ as f64;
                let level = if loading || i < 0.0 || levels.is_empty() {
                    0.0
                } else {
                    levels[(i as usize).min(levels.len() - 1)]
                };
                // More contrast than the raw level, and a fixed spike per slot, for
                // the spiky look of the mock-up.
                let level = (level / 0.7).clamp(0.0, 1.0).powf(0.9)
                    * (0.45 + 0.55 * spike(last - j as i64));
                let env = (std::f32::consts::PI * u).sin().powf(0.9) * smoothstep(0.03, 0.2, u)
                    * smoothstep(0.97, 0.8, u);
                let half = (1.3 * s + level.min(1.2) * half_max * env * open) * settle
                    + 1.3 * s * (1.0 - settle);
                let mut hue = neon(u);
                let mut alpha = open * gone;
                // Loading: a bright run moves along the dotted line.
                if loading {
                    let head = fraction.unwrap_or(((t - shown) * 0.5).fract() as f32);
                    let d = (u - head).abs();
                    alpha *= 0.45 + 0.55 * (1.0 - smoothstep(0.0, 0.05, d));
                }
                // Finishing: grey-violet right of the playhead.
                if finishing {
                    let head = fraction.unwrap_or(0.0);
                    if u > head {
                        hue = lerp(hue, Color::hex(0x5a4a8a, 1.0), 0.75);
                        alpha *= 0.7;
                    }
                }
                let r = RRect::new(
                    Rect::new(x - bar_w / 2.0, cy - half, bar_w, 2.0 * half),
                    bar_w / 2.0,
                );
                p.shape(ShapeInstance::rrect(r, Paint::solid(hue.fade(alpha))));
                // A paler core makes the bar read as a lit tube.
                let core = bar_w * 0.4;
                let rc = RRect::new(
                    Rect::new(x - core / 2.0, cy - half + 0.5, core, (2.0 * half - 1.0).max(0.0)),
                    core / 2.0,
                );
                p.shape(ShapeInstance::rrect(
                    rc,
                    Paint::solid(lerp(hue, Color::WHITE, 0.35).fade(alpha * 0.35)),
                ));
            }
            if finishing {
                if let Some(head) = fraction {
                    let x = rect.x + wx0 + head * (wx1 - wx0);
                    let r = RRect::new(Rect::new(x - 1.2, cy - 16.0 * s, 2.4, 32.0 * s), 1.2);
                    p.shape(ShapeInstance::rrect(
                        r,
                        Paint::solid(hex(CYAN).fade(gone)),
                    ));
                }
            }
        })
        .size(w, bottom);
        Element::layer("lights")
            .size(w, bottom + 4.0)
            .blend(BlendMode::Add)
            .bloom(0.45, 6.0)
            .child(at(grid, 0.0, 0.0))
            .child(at(wave, 0.0, 0.0))
    }

    /// Once the final text lands: a ring and a check draw on where the
    /// waveform was.
    fn sw_done_mark(&self) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        let s = self.scale();
        let side = 84.0 * s;
        let x = (self.look.width - side) / 2.0;
        let y = WAVE_Y * s - side / 2.0;
        let layer = Element::layer("done-light")
            .size(side, side)
            .blend(BlendMode::Add)
            .bloom(0.8, 6.0);
        if self.extra.final_at.is_none() || self.cancelled() {
            return at(layer, x, y);
        }
        let ring = RING.get_or_init(|| {
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(32.0, 32.0), 29.0)],
            })
        });
        let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
        let style = |w: f32| {
            StrokeStyle::solid(w, hex(MAGENTA))
                .linear(
                    Point::new(0.0, 0.0),
                    hex(MAGENTA),
                    Point::new(side, side),
                    hex(CYAN),
                )
                .glow(4.0)
        };
        let inner = side * 0.5;
        at(
            layer
                .child(at(
                    Element::stroke_path("ring", ring.clone(), Rect::new(0.0, 0.0, 64.0, 64.0), style(2.6))
                        .size(side, side)
                        .motion(Prop::Trim, Spring::critical_to_98(0.3))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    0.0,
                ))
                .child(at(
                    Element::stroke_path(
                        "check",
                        check.clone(),
                        Rect::new(0.0, 0.0, 24.0, 24.0),
                        style(3.4),
                    )
                    .size(inner, inner)
                    .motion(Prop::Trim, Spring::critical_to_98(0.56))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    (side - inner) / 2.0,
                    (side - inner) / 2.0,
                )),
            x,
            y,
        )
    }

    /// The chrome title: letters cut from a chrome gradient, over a magenta
    /// neon copy of themselves, tracking in on every state change.
    fn sw_title(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let f = fonts(&self.look.fonts);
        let px = TITLE_PX * s;
        let line = self.look.fonts.line_height(f.title, px).unwrap_or(px * 1.3);
        let style = TextStyle::new(f.title, px, line).tracking(3.0 * s);
        let box_h = line;
        let box_y = TITLE_Y * s - box_h / 2.0;
        let shown = self.now() >= self.shown_at + TITLE_DELAY;
        let word = self.title_word();
        let cancelled = self.cancelled();
        let shaped = self.measure.layout(word, &style, None);
        let total = shaped.size.w;
        let x0 = (w - total) / 2.0;
        let mid = (word.len() as f32 - 1.0) / 2.0;
        let letters = |layer: &'static str| {
            let mut out = Vec::new();
            if !shown {
                return out;
            }
            for (i, ch) in word.chars().enumerate() {
                let Some(g) = shaped.glyphs.iter().find(|g| g.cluster as usize == i) else {
                    continue;
                };
                let d = (i as f32 - mid) / mid.max(1.0);
                // A cancel shows for a moment only, so its title barely tracks.
                let spread = d * if cancelled { 0.08 } else { 0.6 } * total / 2.0;
                let secs = 0.28 + 0.14 * d.abs() as f64;
                out.push(at(
                    Element::label(format!("{layer}-{word}-{i}"), ch.to_string())
                        .text_style(style)
                        .color(Color::WHITE)
                        .motion(Prop::Offset, Spring::critical_to_98(secs))
                        .motion(Prop::Opacity, Spring::critical_to_98(0.14))
                        .presence(
                            Transition::fade().offset(spread, 0.0),
                            Transition::fade().offset(spread * 0.3, -10.0 * s),
                        ),
                    x0 + g.pos.x,
                    0.0,
                ));
            }
            out
        };
        // Chrome: white above the caps' middle fading to lavender, a white
        // horizon, then deep violet into hot pink below.
        let cap_top = box_h * 0.22;
        let horizon = box_h * 0.53;
        let cap_bottom = box_h * 0.82;
        let (top0, top1, low0, low1) = if cancelled {
            (0xfff0f4, 0xff9ab8, 0x7a0628, 0xff3b6b)
        } else {
            (0xffffff, 0xbfa2ff, 0x3a0b9e, 0xff5ee8)
        };
        let band = |key: &'static str, y: f32, h: f32, a: u32, b: u32| {
            at(
                Element::shape(key, ShapeKind::RRect)
                    .size(w, h)
                    .fill_gradient(hex(a), hex(b))
                    .motion(Prop::Fill, Spring::SOFT),
                0.0,
                y,
            )
        };
        let chrome = Element::layer("title-chrome")
            .size(w, box_h)
            .child(band("sky", 0.0, cap_top, top0, top0))
            .child(band("top", cap_top, horizon - cap_top, top0, top1))
            .child(band("low", horizon, cap_bottom - horizon, low0, low1))
            .child(band("floor", cap_bottom, box_h - cap_bottom, low1, low1))
            .child(at(
                Element::shape("horizon", ShapeKind::RRect)
                    .size(w, 1.6 * s)
                    .fill(Color::hex(0xffffff, 0.95)),
                0.0,
                horizon - 0.8 * s,
            ))
            .child(at(
                Element::shape("tint-l", ShapeKind::RRect)
                    .size(w / 2.0, box_h)
                    .fill(Color::hex(MAGENTA, 0.12)),
                0.0,
                0.0,
            ))
            .child(at(
                Element::shape("tint-r", ShapeKind::RRect)
                    .size(w / 2.0, box_h)
                    .fill(Color::hex(CYAN, if cancelled { 0.0 } else { 0.12 })),
                w / 2.0,
                0.0,
            ))
            .child(Element::mask("letters").children(letters("chrome")));
        let glow_hue = if cancelled { 0xff2b5e } else { MAGENTA };
        let glow = Element::layer("title-glow")
            .size(w, box_h)
            .blend(BlendMode::Add)
            .bloom(0.55, 4.0 * s)
            .children(
                letters("glow")
                    .into_iter()
                    .map(|e| e.color(Color::hex(glow_hue, 0.8))),
            );
        at(
            Element::layer("title")
                .size(w, box_h)
                .child(at(glow, 0.0, 2.0 * s))
                .child(at(chrome, 0.0, 0.0)),
            0.0,
            box_y,
        )
    }

    /// The transcript as centred lines. Each line is its own live text, so
    /// a word that wraps leaves one line and rises into the next.
    fn sw_text(&self) -> Element<Msg> {
        let l = &self.look;
        let s = self.scale();
        let style = text_style(l);
        let width = text_width(l);
        let joined: Vec<&str> = self.shown.iter().map(|w| w.text.as_str()).collect();
        let text = joined.join(" ");
        let shaped = self.measure.layout(&text, &style, Some(width));
        // The line of each word, from where its first glyph landed.
        let mut lines: Vec<Vec<Word>> = Vec::new();
        let mut offset = 0usize;
        let first_y = shaped.glyphs.first().map_or(0.0, |g| g.pos.y);
        for word in &self.shown {
            let y = shaped
                .glyphs
                .iter()
                .find(|g| g.cluster as usize >= offset)
                .map_or(first_y, |g| g.pos.y);
            let line = ((y - first_y) / style.line_height).round().max(0.0) as usize;
            while lines.len() <= line {
                lines.push(Vec::new());
            }
            lines[line].push(word.clone());
            offset += word.text.len() + 1;
        }
        let dim = if self.cancelled() { 0.3 } else { 1.0 };
        let mut column = Element::column("text")
            .width(l.width)
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.4));
        for (i, words) in lines.iter().enumerate() {
            let words = Element::live_text(format!("line-{i}"), words)
                .text_style(style)
                .color(hex(WHITE))
                .text_halo(Color::hex(0x6a5cff, 0.9), 11.0 * s)
                .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                .motion(Prop::Offset, Spring::POP)
                .motion(Prop::Position, Spring::critical_to_98(0.2))
                .word_enter(Transition::fade().offset(0.0, 8.0 * s))
                .word_change(Transition::fade().opacity(0.3).offset(0.0, -5.0 * s));
            column = column.child(
                Element::row(format!("row-{i}"))
                    .width(l.width)
                    .height(style.line_height)
                    .justify(JustifyContent::CENTER)
                    .child(words),
            );
        }
        at(column, 0.0, HEAD * s)
    }

    /// Icon, state and detail, centred near the bottom edge.
    fn sw_status(&self, card_h: f32) -> Element<Msg> {
        let l = &self.look;
        let s = self.scale();
        let f = fonts(&l.fonts);
        let status = self.status();
        let px = STATUS_PX * s;
        let line = l.fonts.line_height(f.status, px).unwrap_or(px * 1.3);
        let style = TextStyle::new(f.status, px, line);
        let blue = hex(BLUE);
        let state = if self.cancelled() {
            "Cancelled".to_string()
        } else if self.extra.final_at.is_some() {
            "Pasted".to_string()
        } else {
            status.state.clone()
        };
        let label = Element::label(format!("state-{state}"), state.as_str())
            .text_style(style)
            .color(blue)
            .text_halo(Color::hex(BLUE, 0.35), 4.0 * s)
            .presence(
                Transition::fade().offset(0.0, 7.0 * s),
                Transition::fade().offset(0.0, -7.0 * s),
            );
        let detail = status
            .detail
            .trim_start_matches(|c: char| c.is_whitespace() || c == '·')
            .to_string();
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .gap(10.0 * s)
            .child(self.sw_icon(&status))
            .child(label);
        // While a model loads, three dots light one by one after the label.
        if status.loading {
            let dots = (0..3).map(|i| {
                Element::label(format!("dot-{i}"), ".")
                    .text_style(style)
                    .color(blue)
                    .opacity(0.0)
                    .ambient(LoopProp::Opacity, Loop::step_on(i, 3, DOTS_SECS))
            });
            row = row.child(
                Element::row("loading-dots")
                    .margin_edges(0.0, 0.0, 0.0, -8.0 * s)
                    .children(dots),
            );
        }
        // Once pasted, the finishing detail ("0 s of audio left") is stale.
        let pasted = self.extra.final_at.is_some();
        if !detail.is_empty() && !self.cancelled() && !pasted {
            let base = state
                .bytes()
                .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
                << 6;
            let words: Vec<Word> = detail
                .split_whitespace()
                .enumerate()
                .map(|(i, w)| Word {
                    id: base | i as u32 & 63,
                    text: w.to_string(),
                })
                .collect();
            row = row
                .child(
                    Element::shape("sep", ShapeKind::Capsule)
                        .size(4.0 * s, 4.0 * s)
                        .fill(blue)
                        .margin_edges(0.0, 4.0 * s, 0.0, 4.0 * s),
                )
                .child(
                    Element::live_text(format!("detail-{state}"), &words)
                        .text_style(style)
                        .color(blue)
                        .text_halo(Color::hex(BLUE, 0.35), 4.0 * s)
                        .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                        .motion(Prop::Offset, Spring::POP)
                        .word_enter(Transition::fade().offset(0.0, 7.0 * s))
                        .word_change(Transition::fade().opacity(0.2).offset(0.0, -7.0 * s))
                        .presence(Transition::fade(), Transition::fade()),
                );
        }
        let row_h = 30.0 * s;
        at(
            Element::row("status-row")
                .size(l.width, row_h)
                .justify(JustifyContent::CENTER)
                .align_items(AlignItems::CENTER)
                .child(row),
            0.0,
            card_h - STATUS_UP * s - row_h / 2.0,
        )
        .motion(Prop::Position, Spring::critical_to_98(GROW_SECS))
    }

    /// Microphone while recording; a stop square in a spinning ring while
    /// the final text is made; a check in a ring when done; an X when
    /// cancelled.
    fn sw_icon(&self, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let s = self.scale();
        let side = 30.0 * s;
        let icon_view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let ring_view = Rect::new(0.0, 0.0, 26.0, 26.0);
        let ring = RING.get_or_init(|| {
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(13.0, 13.0), 11.5)],
            })
        });
        let style = |w: f32| {
            StrokeStyle::solid(w, hex(MAGENTA))
                .linear(
                    Point::new(0.0, 0.0),
                    hex(MAGENTA),
                    Point::new(0.0, side),
                    hex(VIOLET),
                )
                .glow(1.0)
        };
        let slot = Element::layer("icon")
            .size(side, side)
            .blend(BlendMode::Add)
            .bloom(0.3, 3.0);
        let done = self.extra.final_at.is_some();
        let stopped = !done && (status.finishing || self.extra.finish_at.is_some());
        if self.cancelled() {
            let cross = CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)));
            return slot.child(at(
                Element::stroke_path("x", cross.clone(), icon_view, style(2.0))
                    .size(side * 0.7, side * 0.7)
                    .motion(Prop::Trim, Spring::critical_to_98(0.3))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                side * 0.15,
                side * 0.15,
            ));
        }
        if done {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot
                .child(at(
                    Element::stroke_path("ring-closed", ring.clone(), ring_view, style(1.6))
                        .size(side, side)
                        .motion(Prop::Trim, Spring::critical_to_98(0.25))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    0.0,
                ))
                .child(at(
                    Element::stroke_path("check", check.clone(), icon_view, style(2.4))
                        .size(side * 0.55, side * 0.55)
                        .motion(Prop::Trim, Spring::critical_to_98(0.5))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    side * 0.225,
                    side * 0.225,
                ));
        }
        let mut slot = slot;
        if stopped && !self.ended() {
            slot = slot.child(at(
                Element::stroke_path(
                    "ring-spin",
                    ring.clone(),
                    ring_view,
                    StrokeStyle::solid(1.2, hex(VIOLET).fade(0.4)),
                )
                .size(side, side)
                .light_head(0.3, 0.8, hex(CYAN), 3.0)
                .presence(Transition::fade().scale(0.6), Transition::fade()),
                0.0,
                0.0,
            ));
        }
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 64))
        });
        let icon = if stopped { side * 0.4 } else { side };
        slot.child(at(
            Element::morph("mic", morph.clone(), icon_view, style(1.8))
                .size(icon, icon)
                .morph_to(if stopped { 1.0 } else { 0.0 })
                .motion(Prop::Morph, Spring::critical_to_98(0.3))
                .presence(Transition::fade(), Transition::fade().scale(0.6)),
            (side - icon) / 2.0,
            (side - icon) / 2.0,
        ))
    }
}
