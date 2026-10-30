//! The `radial` look of the vo popup: a ring of spectrum bars round a round
//! microphone badge, blue on the left to violet on the right, over a dark
//! translucent card that holds the transcript in a plain medium sans.
//!
//! While listening the bars are the voice's spectrum: the mel bands of the
//! newest frames, low bands at the top and high ones at the bottom, mirrored
//! left and right. With an older daemon that sends no voice features they
//! are built from the level history instead, the newest level at the top
//! running down both sides. Once the recording stops the bars sink to short
//! even ticks and a progress arc with a bright head runs round the badge.
//! When the text lands the bars spring out into an even sunburst and a check
//! draws in the badge.
//!
//! Geometry is laid out in the pixels of the concept image, from the card's
//! left edge and the top of the image, and scaled so the card is as wide as
//! the popup.

use super::*;

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};
use vo::scene::path::{icons, make, Contour, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use voxtype::audio::features::MEL_BANDS;

/// Width of the card in the concept, in concept pixels.
const CONCEPT_W: f32 = 655.0;
/// Room round the popup for the glow, in logical pixels.
const MARGIN: f32 = 16.0;

// Layout in concept pixels.
const RING_X: f32 = 328.0;
const RING_Y: f32 = 195.0;
/// The badge circle, and where the bars start outside it.
const BADGE_R: f32 = 88.0;
const BAR_R: f32 = 104.0;
/// Bar lengths: the ticks at rest and the longest a bar grows.
const BAR_MIN: f32 = 7.0;
const BAR_MAX: f32 = 78.0;
const BAR_W: f32 = 5.0;
/// Bars round the ring, an even number so the two halves mirror.
const BARS: usize = 64;
const CARD_TOP: f32 = 363.0;
const RADIUS: f32 = 26.0;
const TEXT_X: f32 = 50.0;
const TEXT_TOP: f32 = 392.0;
const LINE: f32 = 47.7;
const TEXT_PX: f32 = 36.4;
const RULE_GAP: f32 = 42.0;
const STATUS_GAP: f32 = 34.0;
const BOTTOM_GAP: f32 = 38.0;
const STATE_PX: f32 = 19.0;
const DETAIL_PX: f32 = 17.0;

/// Seconds of level history from the top of the ring to the bottom.
const HISTORY_SECS: f32 = 0.6;
/// Seconds for the bars to grow out when the popup opens.
const OPEN_SECS: f64 = 0.5;
/// Seconds for the bars to sink once the recording stops.
const SINK_SECS: f64 = 0.45;
/// Seconds for the sunburst when the text lands.
const BURST_SECS: f64 = 0.7;
/// How fast the bands follow the voice, per second, up and down.
const ATTACK: f32 = 30.0;
const RELEASE: f32 = 7.0;

const BLUE: u32 = 0x3d7bff;
const VIOLET: u32 = 0xb44cff;
const ICE: u32 = 0xb8ceff;
const CARD_HI: u32 = 0x1a1e4c;
const CARD_LO: u32 = 0x0e112e;
const TEXT: u32 = 0xe8edff;
const DETAIL: u32 = 0x9aa6d6;

fn hex(rgb: u32, a: f32) -> Color {
    Color::hex(rgb, a)
}

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

/// The bars' colour at direction `(x, y)` from the centre, given as
/// `x + y`: blue to the top left, violet to the bottom right.
fn ramp(along: f32, a: f32) -> Color {
    let f = ((along / std::f32::consts::SQRT_2) * 0.5 + 0.5).clamp(0.0, 1.0);
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
    let mix = |s: u32| (ch(BLUE, s) * (1.0 - f) + ch(VIOLET, s) * f).round() as u32;
    hex((mix(16) << 16) | (mix(8) << 8) | mix(0), a)
}

/// A fixed pseudo-random number in 0..1 for bar `i`.
fn seed(i: usize) -> f32 {
    let x = (i as u32).wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0x5bd1_e995;
    (x.wrapping_mul(0x2c1b_3c6d) >> 8) as f32 / (1u32 << 24) as f32
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

/// Fonts for the transcript, the state and the detail.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    state: u32,
    detail: u32,
}

/// Noto Sans, the closest free match to the concept's face, or the look's
/// own sans when it is not installed.
fn fonts(look: &Look) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |family: &str| look.fonts.add_system(family).unwrap_or(SANS);
        Fonts {
            text: add("Noto Sans:medium"),
            state: add("Noto Sans:medium"),
            detail: add("Noto Sans:regular"),
        }
    })
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

    /// Bottom of the card, in concept pixels.
    fn bottom(lines: u32) -> f32 {
        TEXT_TOP + lines as f32 * LINE + RULE_GAP + STATUS_GAP + BOTTOM_GAP
    }

    fn surface(&self) -> Size {
        let h = Geo::bottom(swiss::MAX_LINES as u32) * self.k + 2.0 * MARGIN;
        Size::new((self.w + 2.0 * MARGIN).ceil(), h.ceil())
    }
}

/// State only the radial look keeps.
#[derive(Default)]
pub struct State {
    /// When the dictation ended without a final text.
    ended_at: Option<f64>,
    /// The mel bands, smoothed, 0 to 1, and when they were last moved.
    bands: [f32; MEL_BANDS],
    bands_at: f64,
    /// Whether the daemon sends voice features at all.
    voiced: bool,
}

/// A mel band byte as 0 to 1: -60 dB and below is 0, -15 dB and above is 1.
fn band_level(b: u8) -> f32 {
    let db = b as f32 / 255.0 * voxtype::audio::features::MEL_DB_RANGE
        - voxtype::audio::features::MEL_DB_RANGE;
    ((db + 60.0) / 45.0).clamp(0.0, 1.0)
}

/// What the canvas draws, fixed for one view.
struct Ring {
    k: f32,
    /// The card's bottom edge in concept pixels.
    bottom: f32,
    shown_at: f64,
    /// When the bars start to sink.
    sink_at: Option<f64>,
    final_at: Option<f64>,
    ended: bool,
    /// The final pass's or the model load's progress, when it is known.
    progress: Option<f32>,
    loading: bool,
    /// The smoothed mel bands, when the daemon sends them.
    bands: Option<[f32; MEL_BANDS]>,
    wave: Arc<[f32]>,
    newest_at: f64,
    rate: f32,
}

impl Ring {
    fn p(&self, x: f32, y: f32) -> Point {
        Point::new(x * self.k, y * self.k)
    }

    /// The level shown at `age` seconds before now, 0 to 1.
    fn level_at(&self, t: f64, age: f32) -> f32 {
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

    /// How far bar `i` reaches while listening, 0 to 1. `d` runs from 0 at
    /// the top of the ring to 1 at the bottom.
    fn reach(&self, t: f64, i: usize, d: f32) -> f32 {
        let s = seed(i.min(BARS - i));
        let wobble = 0.82 + 0.18 * (t as f32 * (3.0 + 2.0 * s) + s * TAU).sin();
        match &self.bands {
            Some(bands) => {
                let x = d * (MEL_BANDS - 1) as f32;
                let (a, b) = (x.floor() as usize, x.ceil() as usize);
                let f = x - a as f32;
                let v = bands[a] * (1.0 - f) + bands[b] * f;
                (v * (0.45 + 0.55 * s) * wobble).min(1.0)
            }
            None => {
                // Speech carries most energy low, so the top bars run long.
                let shape = 1.0 - 0.55 * d;
                let level = (self.level_at(t, d * HISTORY_SECS) * 5.0).min(1.0);
                (level * shape * (0.3 + 0.7 * s) * wobble).min(1.0)
            }
        }
    }

    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let open = ease_out((t - self.shown_at) / OPEN_SECS);
        let sink = self
            .sink_at
            .map_or(0.0, |s| ease_out((t - s) / SINK_SECS));
        let burst = self.final_at.map(|f| ((t - f) / BURST_SECS).clamp(0.0, 1.0) as f32);
        let fade = if self.ended { 0.4 } else { 1.0 };

        // The card.
        let card = RRect::new(
            Rect::new(0.0, CARD_TOP * k, CONCEPT_W * k, (self.bottom - CARD_TOP) * k),
            RADIUS * k,
        );
        p.shape(
            ShapeInstance::rrect(
                card,
                Paint::linear(
                    self.p(0.0, CARD_TOP),
                    hex(CARD_HI, 0.86),
                    self.p(0.0, self.bottom),
                    hex(CARD_LO, 0.92),
                ),
            )
            .with_stroke(
                Paint::linear(
                    self.p(0.0, CARD_TOP),
                    hex(BLUE, 0.55),
                    self.p(CONCEPT_W, CARD_TOP),
                    hex(VIOLET, 0.75),
                ),
                1.5 * k,
            ),
        );
        // The rule over the status line.
        let ry = self.bottom - BOTTOM_GAP - STATUS_GAP;
        p.shape(ShapeInstance::line(
            self.p(10.0, ry),
            self.p(CONCEPT_W - 10.0, ry),
            1.0 * k,
            Paint::solid(hex(0x3a4280, 0.6)),
        ));

        // The bars: a gradient from the top left to the bottom right.
        let c = self.p(RING_X, RING_Y);
        let paint = |w: f32, a: f32| {
            StrokeStyle::solid(w * k, hex(BLUE, a)).linear(
                self.p(RING_X - 170.0, RING_Y - 160.0),
                hex(BLUE, a),
                self.p(RING_X + 170.0, RING_Y + 160.0),
                hex(VIOLET, a),
            )
        };
        let scan = ((t - self.shown_at) * 0.9).rem_euclid(1.0) as f32;
        for i in 0..BARS {
            let a = i as f32 / BARS as f32 * TAU - PI / 2.0;
            // 0 at the top, 1 at the bottom, the same on both sides.
            let d = {
                let u = i as f32 / BARS as f32;
                (if u <= 0.5 { u } else { 1.0 - u }) * 2.0
            };
            let live = BAR_MIN + (BAR_MAX - BAR_MIN) * self.reach(t, i, d);
            let mut len = match burst {
                Some(b) => {
                    // Out to an even sunburst and settle a little shorter.
                    let even = 30.0 + 16.0 * seed(i.min(BARS - i));
                    let kick = (b * PI).sin() * 22.0 * (1.0 - b);
                    BAR_MIN + (even - BAR_MIN) * ease_out(b as f64 * 2.0) + kick
                }
                None => live * (1.0 - sink) + BAR_MIN * sink,
            };
            len *= open.max(0.05);
            let mut alpha = 1.0;
            if self.loading && self.sink_at.is_none() {
                // A soft sweep round the waiting ticks.
                let u = (i as f32 / BARS as f32 - scan).rem_euclid(1.0);
                alpha = 0.35 + 0.65 * (1.0 - u).powi(6);
            } else if burst.is_none() && sink > 0.0 {
                alpha = 1.0 - 0.55 * sink;
            }
            let (ca, sa) = (a.cos(), a.sin());
            let r0 = BAR_R * k;
            let r1 = (BAR_R + len) * k;
            let bar = Contour::polyline(
                &[
                    Point::new(c.x + ca * r0, c.y + sa * r0),
                    Point::new(c.x + ca * r1, c.y + sa * r1),
                ],
                false,
            );
            let style = StrokeStyle::solid(BAR_W * (1.0 - 0.45 * sink * (1.0 - burst.unwrap_or(0.0))) * k, ramp(ca + sa, alpha * fade)).glow(3.0 * k);
            p.stroke(&bar, &style);
        }

        // The badge circle, and the progress arc on it once known.
        let ring = make::ring(c, BADGE_R * k);
        let rl = ring.length();
        let finishing = self.sink_at.is_some() && self.final_at.is_none() && !self.ended;
        let base = if finishing { 0.35 } else { 0.95 * fade };
        p.stroke(
            &ring.trim(0.0, rl * open),
            &paint(2.4, base).glow(6.0 * k),
        );
        if let Some(f) = self.progress.filter(|_| self.final_at.is_none() && !self.ended) {
            let f = f.clamp(0.0, 1.0);
            p.stroke(&ring.trim(0.0, rl * f), &paint(3.0, 1.0).glow(8.0 * k));
            if let Some((head, _)) = ring.at(rl * f) {
                p.shape(ShapeInstance::circle(
                    head,
                    7.0 * k,
                    Paint::glow(hex(ICE, 0.9), 10.0 * k, 1.2),
                ));
                p.shape(ShapeInstance::circle(head, 3.2 * k, Paint::solid(hex(0xffffff, 1.0))));
            }
        }
        if let Some(b) = burst {
            // One bright pass round the badge as the text lands.
            let a = 1.0 - b;
            p.stroke(&ring, &paint(3.0, a).glow(12.0 * k));
        }
    }
}

/// The transcript's type: Noto Sans Medium, scaled with the popup.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = look.width / CONCEPT_W;
    TextStyle::new(fonts(look).text, TEXT_PX * k, LINE * k)
}

/// Width the transcript wraps at.
pub(crate) fn text_width(look: &Look) -> f32 {
    let k = look.width / CONCEPT_W;
    585.0 * k
}

impl Popup {
    pub(crate) fn radial_surface(look: &Look) -> Size {
        Geo::new(look).surface()
    }

    /// Notes when a dictation ended without a final text, and lets the
    /// bands follow the newest voice features.
    pub(crate) fn sync_radial(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Radial {
            return;
        }
        if self.ended() && self.extra.final_at.is_none() && self.radial.ended_at.is_none() {
            self.radial.ended_at = Some(t);
        }
        let n = self.voice.len();
        if n == 0 {
            return;
        }
        let mut target = [0.0f32; MEL_BANDS];
        let take = n.min(3);
        for v in self.voice.iter().skip(n - take) {
            for (o, b) in target.iter_mut().zip(v.features.mel) {
                *o += band_level(b) / take as f32;
            }
        }
        let dt = (t - self.radial.bands_at).clamp(0.0, 0.1) as f32;
        for (o, x) in self.radial.bands.iter_mut().zip(target) {
            let rate = if x > *o { ATTACK } else { RELEASE };
            *o += (x - *o) * (1.0 - (-rate * dt).exp());
        }
        self.radial.bands_at = t;
        self.radial.voiced = true;
    }

    pub(crate) fn radial_view(&self) -> Element<Msg> {
        let g = Geo::new(&self.look);
        let size = g.surface();
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        let k = g.k;
        let status = self.status();
        let bottom = Geo::bottom(self.lines);
        let done = self.extra.final_at.is_some();
        let finishing = status.finishing && !status.loading;
        let sink_at = self
            .extra
            .finish_at
            .or(self.radial.ended_at)
            .or_else(|| finishing.then_some(self.extra.now));
        let ring = Ring {
            k,
            bottom,
            shown_at: self.shown_at,
            sink_at,
            final_at: self.extra.final_at,
            ended: self.radial.ended_at.is_some(),
            progress: self
                .shows_progress(&status)
                .then(|| self.files.fraction.map(|f| f as f32))
                .flatten(),
            loading: status.loading,
            bands: self.radial.voiced.then_some(self.radial.bands),
            wave: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
        };
        let until = match (self.extra.final_at, self.radial.ended_at) {
            (Some(f), _) => f + BURST_SECS + 0.1,
            (None, Some(e)) => e + SINK_SECS + 0.1,
            _ => f64::INFINITY,
        };
        let canvas = at(
            Element::canvas("ring", until, move |t, r, p| {
                p.layer(vo::scene::geom::Affine::translate(r.x, r.y), 1.0, |p| {
                    ring.draw(t, p)
                })
            }),
            0.0,
            0.0,
        )
        .size(g.w, bottom * k);
        let exit = if done {
            Transition::fade().offset(0.0, -8.0)
        } else {
            Transition::fade().scale(0.97)
        };
        let panel = at(Element::layer("radial"), MARGIN, MARGIN)
            .size(g.w, bottom * k)
            .presence(Transition::fade().opacity(1.0), exit)
            .child(canvas)
            .child(self.radial_badge(&g, &status))
            .child(self.radial_text(&g))
            .children(self.score_beside(
                TEXT_X * g.k,
                TEXT_TOP * g.k,
                self.look.text_width(),
                36.0,
                hex(ICE, 1.0),
            ))
            .child(self.radial_status(&g, &status, bottom));
        root.child(panel)
    }

    /// The microphone, the stop square, the check or the cross in the badge.
    fn radial_badge(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let side = 84.0 * k;
        let place = |e: Element<Msg>, s: f32| {
            at(e.size(s, s), RING_X * k - s / 2.0, RING_Y * k - s / 2.0)
        };
        let slot = at(Element::layer("badge"), 0.0, 0.0);
        let mark = |key: &'static str, o: &Arc<Outline>| {
            Element::stroke_path(
                key,
                o.clone(),
                view,
                StrokeStyle::solid(5.0 * k, hex(ICE, 1.0)).glow(5.0 * k),
            )
            .motion(Prop::Trim, Spring::critical_to_98(0.4))
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
        };
        if self.extra.final_at.is_some() {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot.child(place(mark("check", check), side * 0.9));
        }
        if self.radial.ended_at.is_some() {
            let cross = CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)));
            return slot.child(place(mark("cross", cross), side * 0.7));
        }
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 64))
        });
        let stopped = status.finishing && !status.loading;
        slot.child(place(
            Element::morph(
                "mic",
                morph.clone(),
                view,
                StrokeStyle::solid(5.0 * k, hex(ICE, 1.0)).glow(3.0 * k),
            )
            .morph_to(if stopped { 1.0 } else { 0.0 })
            .motion(Prop::Morph, Spring::critical_to_98(0.3))
            .presence(Transition::fade(), Transition::fade().scale(0.6)),
            if stopped { side * 0.55 } else { side },
        ))
    }

    /// The transcript: words rise into place as they fade in.
    fn radial_text(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let mut words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .text_style(self.look.text_style())
            .color(hex(TEXT, 1.0))
            .width(self.look.text_width())
            .motion(Prop::Opacity, Spring::critical_to_98(0.2))
            .motion(Prop::Offset, Spring::critical_to_98(0.25))
            .motion(Prop::TextFx, Spring::critical_to_98(0.22))
            .word_enter(self.ghost_enter(Transition::fade().offset(0.0, 7.0 * k)));
        if self.radial.ended_at.is_some() {
            words = words.opacity(0.4);
        }
        let words = self.handoff_or(
            words,
            &self.look.text_style(),
            Some(self.look.text_width()),
            hex(TEXT, 1.0),
            hex(ICE, 1.0),
        );
        let (style, width) = (self.look.text_style(), self.look.text_width());
        let text = Element::layer("text")
            .size(width, self.lines.max(1) as f32 * style.line_height)
            .child(words);
        let text = self.signal_marks(text, &self.shown, width, (0.0, 0.0), hex(TEXT, 1.0));
        at(text, TEXT_X * k, TEXT_TOP * k)
    }

    /// The status line: a level icon, a spinner, a check or a cross, then
    /// the state and the detail.
    fn radial_status(&self, g: &Geo, status: &StatusLine, bottom: f32) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let f = fonts(&self.look);
        let done = self.extra.final_at.is_some();
        let cancelled = self.radial.ended_at.is_some();
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
        } else if cancelled {
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
        let state_style = TextStyle::new(f.state, STATE_PX * k, 26.0 * k);
        let detail_style = TextStyle::new(f.detail, DETAIL_PX * k, 26.0 * k);
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!("·  {detail}")
        };
        let wid = |s: &str, st: &TextStyle| self.measure.layout(s, st, None).size.w.ceil() + 2.0;
        let icon_w = 40.0 * k;
        let icon_h = 26.0 * k;
        let mark = |key: &'static str, o: &Arc<Outline>, s: f32| {
            Element::stroke_path(
                key,
                o.clone(),
                Rect::new(0.0, 0.0, 24.0, 24.0),
                StrokeStyle::solid(2.2 * k, hex(BLUE, 1.0)),
            )
            .size(s, s)
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
        };
        let icon = if done || cancelled {
            let s = icon_h * 0.6;
            let o = if done {
                CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)))
            } else {
                CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)))
            };
            Element::layer("icon")
                .size(icon_w, icon_h)
                .child(
                    at(Element::shape("ring", ShapeKind::Circle), 0.0, 0.0)
                        .size(icon_h, icon_h)
                        .fill(Color::TRANSPARENT)
                        .border(hex(BLUE, 1.0), 1.8 * k),
                )
                .child(at(
                    mark(if done { "check" } else { "cross" }, o, s),
                    (icon_h - s) / 2.0,
                    (icon_h - s) / 2.0,
                ))
        } else if finishing_icon(status) {
            let shown_at = self.shown_at;
            Element::canvas("spinner", f64::INFINITY, move |t, r, p| {
                let c = Point::new(r.x + r.h / 2.0, r.y + r.h / 2.0);
                let turn = ((t - shown_at) * 1.4).rem_euclid(1.0) as f32;
                for i in 0..8 {
                    let a = i as f32 / 8.0 * TAU - PI / 2.0;
                    let lag = (i as f32 / 8.0 - turn).rem_euclid(1.0);
                    let alpha = 0.25 + 0.75 * lag;
                    let col = if i < 4 { BLUE } else { VIOLET };
                    p.shape(ShapeInstance::circle(
                        Point::new(c.x + a.cos() * r.h * 0.38, c.y + a.sin() * r.h * 0.38),
                        2.0 * k,
                        Paint::solid(hex(col, alpha)),
                    ));
                }
            })
            .size(icon_w, icon_h)
        } else {
            // Five bars that follow the voice level.
            let wave = self.wave.clone();
            let newest_at = self.newest_at;
            Element::canvas("level", f64::INFINITY, move |t, r, p| {
                let n = wave.len();
                let lvl = if n == 0 {
                    0.0
                } else {
                    let back = ((newest_at - t) * FRAME_HZ as f64).max(0.0) as usize;
                    wave[n - 1 - back.min(n - 1)]
                };
                for i in 0..5 {
                    let shape = [0.45, 0.8, 1.0, 0.7, 0.5][i];
                    let jit = 0.75 + 0.25 * (t as f32 * 7.0 + i as f32 * 1.9).sin();
                    let h = r.h * (0.3 + 0.7 * (lvl * 2.4).min(1.0) * shape * jit);
                    let x = r.x + (4.0 + i as f32 * 6.5) * k;
                    let cy = r.y + r.h / 2.0;
                    let col = if i < 3 { BLUE } else { VIOLET };
                    p.shape(ShapeInstance::line(
                        Point::new(x, cy - h / 2.0),
                        Point::new(x, cy + h / 2.0),
                        3.0 * k,
                        Paint::solid(hex(col, 1.0)),
                    ));
                }
            })
            .size(icon_w, icon_h)
        };
        let row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .gap(12.0 * k)
            .child(icon)
            .child(
                Element::label(format!("state-{state}"), state.as_str())
                    .text_style(state_style)
                    .color(hex(TEXT, 1.0))
                    .width(wid(&state, &state_style))
                    .presence(Transition::fade().offset(0.0, 6.0 * k), Transition::fade()),
            )
            .child(
                Element::label(format!("detail-{state}"), detail.as_str())
                    .text_style(detail_style)
                    .color(hex(DETAIL, 1.0))
                    .width(wid(&detail, &detail_style))
                    .presence(Transition::fade().offset(0.0, 6.0 * k), Transition::fade()),
            );
        let y = (bottom - BOTTOM_GAP) * k - icon_h / 2.0;
        at(row, 42.0 * k, y)
    }
}

/// Whether the status line shows the spinner rather than the level bars.
fn finishing_icon(status: &StatusLine) -> bool {
    status.finishing || status.loading
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(bands: Option<[f32; MEL_BANDS]>) -> Ring {
        Ring {
            k: 1.0,
            bottom: 700.0,
            shown_at: 0.0,
            sink_at: None,
            final_at: None,
            ended: false,
            progress: None,
            loading: false,
            bands,
            wave: Arc::from([0.0f32; 200].as_slice()),
            newest_at: 1.0,
            rate: 100.0,
        }
    }

    #[test]
    fn the_bars_mirror_left_and_right() {
        let r = ring(Some([0.6; MEL_BANDS]));
        for i in 1..BARS / 2 {
            let d = 2.0 * i as f32 / BARS as f32;
            let left = r.reach(1.0, BARS - i, d);
            let right = r.reach(1.0, i, d);
            assert!((left - right).abs() < 1e-5, "bar {i}: {left} vs {right}");
        }
    }

    #[test]
    fn silence_keeps_the_bars_short_and_a_loud_low_band_lifts_the_top() {
        let quiet = ring(Some([0.0; MEL_BANDS]));
        assert_eq!(quiet.reach(1.0, 0, 0.0), 0.0);
        let mut bands = [0.0; MEL_BANDS];
        bands[0] = 1.0;
        let low = ring(Some(bands));
        assert!(low.reach(1.0, 0, 0.0) > 0.5);
        assert_eq!(low.reach(1.0, BARS / 2, 1.0), 0.0);
        assert_eq!(band_level(0), 0.0);
        assert_eq!(band_level(255), 1.0);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use voxtype::osd::config::OsdTheme;

    /// M7.L2: Radial edits in its own layout, its text box at its `k` scale. With the caret on the
    /// newest, a middle and the top line, at both widths: the caret on its
    /// glyph within 1 px in every frame, the popup's size unchanged, and the
    /// golden `tests/fixtures/osd-vo/edit-radial.txt`.
    #[test]
    fn radial_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Radial, "radial");
    }

    /// M7.L2: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn radial_edits_within_the_envelope() {
        envelope(OsdTheme::Radial, "radial");
    }
}
