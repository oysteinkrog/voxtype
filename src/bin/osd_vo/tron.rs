//! The `tron` look of the vo popup: a black glass panel traced by cyan light
//! lines. The border has chamfered corners, a gap in the top edge for a round
//! microphone badge and a step in the bottom edge. The waveform is one
//! luminous line with two fainter echoes, and the transcript is set in a thin
//! geometric face.
//!
//! The border and the badge draw themselves on from the top when the popup
//! opens. While listening a light head with a fading tail runs round the
//! border. The waveform is rebuilt from the level history every frame: the
//! newest level in the middle, older ones toward both ends. Once the
//! recording stops the line relaxes to flat and the light head becomes a
//! progress trace that fills the border with the final pass. When the text
//! lands the whole border flares, the line glows flat and a check draws in
//! the badge.
//!
//! Geometry is laid out in the pixels of the concept image and scaled so the
//! panel is as wide as the popup.

use super::*;

use std::f32::consts::TAU;
use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::Point;
use vo::scene::path::{icons, make, Contour, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Width of the panel in the concept, in concept pixels.
const CONCEPT_W: f32 = 712.0;
/// Room round the popup for the glow, in logical pixels.
const MARGIN: f32 = 18.0;

// Layout in concept pixels from the top of the badge's outer ring and the
// panel's left edge.
const PANEL_TOP: f32 = 89.0;
const BADGE_X: f32 = 356.0;
const BADGE_Y: f32 = 62.0;
const BADGE_R: f32 = 42.0;
const WAVE_Y: f32 = 185.0;
const WAVE_X0: f32 = 25.0;
const WAVE_X1: f32 = 686.0;
const WAVE_AMP: f32 = 36.0;
const TEXT_X: f32 = 45.0;
const TEXT_TOP: f32 = 240.0;
const LINE: f32 = 50.0;
const TEXT_PX: f32 = 36.0;
const RULE_GAP: f32 = 20.0;
const STATUS_GAP: f32 = 29.0;
const BOTTOM_GAP: f32 = 35.0;
const STATUS_PX: f32 = 17.5;
/// Corner chamfer and the depth of the steps in the top and bottom edges.
const CHAMFER: f32 = 22.0;
const STEP: f32 = 10.0;

/// Seconds for the border to draw itself on.
const OPEN_SECS: f64 = 0.55;
/// Seconds for the light head to run once round the border.
const LAP_SECS: f64 = 3.6;
/// Length of the head's tail, as a fraction of the border.
const TAIL: f32 = 0.16;
/// Seconds of level history from the middle of the line to each end.
const HISTORY_SECS: f32 = 1.6;
/// Seconds for the line to relax once the recording stops.
const RELAX_SECS: f64 = 0.5;
/// Seconds of the flare when the text lands.
const FLARE_SECS: f64 = 0.6;

/// With `light-line` (MO-68): at each onset the light head leaves the
/// border, draws a line under the new word and goes back.
pub(crate) const LIGHT_LINE: &str = "light-line";
/// Seconds for the head to reach the word at the least, to draw the line
/// under it, to hold it and to go back to the border.
const LIGHT_LEAVE: f64 = 0.18;
const LIGHT_DRAW: f64 = 0.22;
const LIGHT_HOLD: f64 = 0.2;
const LIGHT_BACK: f64 = 0.25;

const CYAN: u32 = 0x29d3ff;
const BRIGHT: u32 = 0xb5f3ff;
const DIM: u32 = 0x0f6f99;
const GLASS: u32 = 0x02080f;
const TEXT: u32 = 0xc9efff;
const DETAIL: u32 = 0x45bfe8;

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

/// Fonts for the transcript and the status line.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    status: u32,
}

/// Saira, the thin squarish face closest to the concept's, or the look's
/// own sans when it is not installed.
fn fonts(look: &Look) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |family: &str| look.fonts.add_system(family).unwrap_or(SANS);
        Fonts {
            text: add("Saira:light"),
            status: add("Saira:regular"),
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

    /// Bottom of the panel below the top of the badge, in concept pixels.
    fn bottom(lines: u32) -> f32 {
        TEXT_TOP + lines as f32 * LINE + RULE_GAP + STATUS_GAP + BOTTOM_GAP
    }

    fn surface(&self) -> Size {
        let h = Geo::bottom(swiss::MAX_LINES as u32) * self.k + 2.0 * MARGIN;
        Size::new((self.w + 2.0 * MARGIN).ceil(), h.ceil())
    }
}

/// The border as one closed contour in concept pixels, starting at the top
/// middle and running clockwise, so a trim from 0 grows from the badge.
fn border(bottom: f32) -> Contour {
    let (w, top, c, s) = (CONCEPT_W, PANEL_TOP, CHAMFER, STEP);
    let gap = 66.0;
    let (l, r) = (BADGE_X - gap, BADGE_X + gap);
    let pts = [
        Point::new(r, top + s),
        Point::new(r + s, top),
        Point::new(w - c, top),
        Point::new(w, top + c),
        Point::new(w, bottom - c - s),
        Point::new(w - c, bottom - s),
        Point::new(w - 140.0, bottom - s),
        Point::new(w - 140.0 - s, bottom),
        Point::new(140.0 + s, bottom),
        Point::new(140.0, bottom - s),
        Point::new(c, bottom - s),
        Point::new(0.0, bottom - c - s),
        Point::new(0.0, top + c),
        Point::new(c, top),
        Point::new(l - s, top),
        Point::new(l, top + s),
    ];
    Contour::polyline(&pts, false)
}

/// State only the tron look keeps.
#[derive(Default)]
pub struct State {
    /// When the dictation ended without a final text.
    ended_at: Option<f64>,
}

/// What the canvas draws, fixed for one view.
struct Lines {
    k: f32,
    /// The panel's bottom edge in concept pixels.
    bottom: f32,
    shown_at: f64,
    /// When the waveform starts to relax.
    relax_at: Option<f64>,
    final_at: Option<f64>,
    ended: bool,
    /// The final pass's progress, when it is known.
    progress: Option<f32>,
    loading: bool,
    wave: Arc<[f32]>,
    newest_at: f64,
    rate: f32,
    /// The waveform's clock during a rewind cancel (MO-50).
    scrub: endings::Scrub,
    /// The light line under the newest word (MO-68).
    light: Option<LightLine>,
}

/// MO-68: the light head's trip from the border to the newest word and
/// back, in the canvas's logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LightLine {
    /// The onset that sent it, or the word's arrival without one.
    from: f64,
    /// When the head reaches the word: the word must be there.
    landed: f64,
    x0: f32,
    x1: f32,
    y: f32,
    reduced: bool,
}

/// Where a light line's trip is at `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Trip {
    /// Not yet left, or back on the border.
    Home,
    /// Flying to the word's left end, 0 to 1.
    Out(f32),
    /// Drawing the line, 0 to 1 of its length.
    Draw(f32),
    Hold,
    /// Flying back to the border, 0 to 1; the line fades.
    Back(f32),
}

impl LightLine {
    fn trip(&self, t: f64) -> Trip {
        let k = |a: f64, d: f64| ((t - a) / d).clamp(0.0, 1.0) as f32;
        let drawn = self.landed + LIGHT_DRAW;
        let held = drawn + LIGHT_HOLD;
        if t < self.from || t >= held + LIGHT_BACK {
            Trip::Home
        } else if t < self.landed {
            Trip::Out(k(self.from, self.landed - self.from))
        } else if t < drawn {
            Trip::Draw(k(self.landed, LIGHT_DRAW))
        } else if t < held {
            Trip::Hold
        } else {
            Trip::Back(k(held, LIGHT_BACK))
        }
    }

    /// Whether the head is off the border at `t`.
    fn away(&self, t: f64) -> bool {
        !self.reduced && self.trip(t) != Trip::Home
    }
}

impl Lines {
    fn p(&self, x: f32, y: f32) -> Point {
        Point::new(x * self.k, y * self.k)
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

    /// The waveform's height above its centre line at `x`, 0 to 1 along it,
    /// for an echo with `phase` and `freq`.
    fn wave_y(&self, t: f64, x: f32, phase: f32, freq: f32, calm: f32) -> f32 {
        let d = (x - 0.5).abs() * 2.0;
        let window = (1.0 - d * d).max(0.0).powf(1.5);
        // A smooth envelope: the level a little back in time, blended with
        // its mean over the history, so the line swells rather than jitters.
        let mean = (0..8)
            .map(|i| self.level_at(t, i as f32 * HISTORY_SECS / 8.0))
            .sum::<f32>()
            / 8.0;
        let near = self.level_at(t, d * HISTORY_SECS * 0.5);
        let level = ((0.6 * mean + 0.4 * near) * 2.2).min(1.0);
        let drift = t as f32 * 1.7;
        let s = (TAU * (x * freq + phase) - drift).sin() * 0.7
            + (TAU * (x * freq * 2.3 + phase * 0.6) + drift * 1.3).sin() * 0.3;
        WAVE_AMP * window * level * calm * s
    }

    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let open = ease_out((t - self.shown_at) / OPEN_SECS);
        let flare = self
            .final_at
            .map_or(0.0, |f| 1.0 - ease_out((t - f) / FLARE_SECS));
        let done = self.final_at.map_or(0.0, |f| ease_out((t - f) / 0.25));
        let fade = if self.ended { 0.45 } else { 1.0 };

        // The glass.
        let edge = border(self.bottom).transform(&vo::scene::geom::Affine::scale(k));
        p.fill([&edge], &StrokeStyle::solid(1.0, hex(GLASS, 0.9 * open)));

        // The border, drawn on from the top middle both ways.
        let len = edge.length();
        let trace_dim = if self.relax_at.is_some() && self.final_at.is_none() { 0.55 } else { 1.0 };
        let base_a = (0.85 * trace_dim + 0.15 * done + 0.3 * flare) * fade;
        let line = StrokeStyle::solid((2.0 + 1.2 * flare) * k, hex(CYAN, base_a.min(1.0)))
            .glow((3.5 + 6.0 * flare) * k);
        p.stroke(&edge.trim(0.0, len * open * 0.5), &line);
        p.stroke(&edge.trim(len * (1.0 - open * 0.5), len), &line);

        // The light: a head with a fading tail running round while
        // listening, a growing trace while the final pass runs. It leaves
        // the border while it draws a light line (MO-68).
        let away = self.light.is_some_and(|l| l.away(t));
        if open >= 1.0 && self.final_at.is_none() && !self.ended && !away {
            match (self.relax_at, self.progress) {
                (Some(_), Some(f)) | (None, Some(f))
                    if !self.loading || self.relax_at.is_some() =>
                {
                    let f = f.clamp(0.0, 1.0);
                    p.stroke(
                        &edge.trim(0.0, len * f),
                        &StrokeStyle::solid(2.2 * k, hex(BRIGHT, 0.9)).glow(4.0 * k),
                    );
                    self.head(p, &edge, len * f);
                }
                _ => {
                    let at = ((t - self.shown_at - OPEN_SECS) / LAP_SECS).rem_euclid(1.0) as f32;
                    let head = len * at;
                    let pieces = 10;
                    for i in 0..pieces {
                        let a = 1.0 - i as f32 / pieces as f32;
                        let s1 = head - len * TAIL * i as f32 / pieces as f32;
                        let s0 = head - len * TAIL * (i + 1) as f32 / pieces as f32;
                        let seg =
                            |a0: f32, a1: f32| edge.trim(a0.rem_euclid(len), a1.rem_euclid(len));
                        if s0.rem_euclid(len) < s1.rem_euclid(len) {
                            p.stroke(
                                &seg(s0, s1),
                                &StrokeStyle::solid(2.8 * k, hex(BRIGHT, 0.9 * a * a))
                                    .glow(6.0 * k),
                            );
                        }
                    }
                    self.head(p, &edge, head);
                }
            }
        }

        // The badge: a bright ring and a thin outer ring with a brighter top arc.
        let c = self.p(BADGE_X, BADGE_Y);
        p.shape(ShapeInstance::circle(
            c,
            BADGE_R * k,
            Paint::solid(hex(GLASS, 0.92 * open)),
        ));
        let ring = make::ring(c, BADGE_R * k);
        let rl = ring.length();
        p.stroke(
            &ring.trim(0.0, rl * open),
            &StrokeStyle::solid((2.4 + flare) * k, hex(CYAN, fade)).glow((6.0 + 6.0 * flare) * k),
        );
        let outer = make::ring(c, 59.0 * k);
        p.stroke(
            &outer.trim(0.0, outer.length() * open),
            &StrokeStyle::solid(1.0 * k, hex(DIM, 0.8 * fade)),
        );
        let turn = if self.relax_at.is_none() {
            (t - self.shown_at) as f32 * 0.6
        } else {
            0.0
        };
        p.stroke(
            &make::arc(c, 59.0 * k, -2.2 + turn, 1.3 * open),
            &StrokeStyle::solid(1.6 * k, hex(CYAN, 0.9 * fade)).glow(4.0 * k),
        );

        // The waveform: one luminous line and two fainter echoes.
        let calm = match self.relax_at {
            Some(r) => 1.0 - 0.9 * ease_out((t - r) / RELAX_SECS),
            None => 1.0,
        } * (1.0 - done);
        let n = 160;
        let trace = |phase: f32, freq: f32, gain: f32| {
            let pts: Vec<Point> = (0..=n)
                .map(|i| {
                    let x = i as f32 / n as f32;
                    let px = WAVE_X0 + (WAVE_X1 - WAVE_X0) * x;
                    self.p(px, WAVE_Y - gain * self.wave_y(t, x, phase, freq, calm))
                })
                .collect();
            Contour::polyline(&pts, false)
        };
        let wl = open;
        for (phase, freq, gain, a) in [(0.31, 6.4, 0.8, 0.4), (0.67, 7.3, 0.6, 0.35)] {
            let c = trace(phase, freq, gain);
            let l = c.length();
            p.stroke(
                &c.trim(l * (0.5 - wl * 0.5), l * (0.5 + wl * 0.5)),
                &StrokeStyle::solid(1.2 * k, hex(CYAN, a * fade)),
            );
        }
        let main = trace(0.0, 7.0, 1.0);
        let l = main.length();
        p.stroke(
            &main.trim(l * (0.5 - wl * 0.5), l * (0.5 + wl * 0.5)),
            &StrokeStyle::solid((1.8 + 0.6 * done) * k, hex(BRIGHT, (0.95 * fade).min(1.0)))
                .glow((3.0 + 1.0 * done) * k),
        );
        if self.final_at.is_none() && !self.ended {
            if let Some(light) = self.light {
                self.light_line(t, &light, &edge, p);
            }
        }
    }

    /// The light head on the border at `t`, where it runs while listening.
    fn head_at(&self, t: f64, edge: &Contour) -> Option<Point> {
        let len = edge.length();
        let at = ((t - self.shown_at - OPEN_SECS) / LAP_SECS).rem_euclid(1.0) as f32;
        edge.at(len * at).map(|(p, _)| p)
    }

    /// MO-68: the head flies from the border to the newest word, draws a
    /// line under it and flies back. Reduced motion shows the line alone.
    fn light_line(&self, t: f64, l: &LightLine, edge: &Contour, p: &mut Painter) {
        let k = self.k;
        let line = |p: &mut Painter, to: f32, a: f32| {
            let pts = [Point::new(l.x0, l.y), Point::new(l.x0 + (l.x1 - l.x0) * to, l.y)];
            p.stroke(
                &Contour::polyline(&pts, false),
                &StrokeStyle::solid(1.6 * k, hex(BRIGHT, a)).glow(4.0 * k),
            );
        };
        if l.reduced {
            if t >= l.landed {
                line(p, 1.0, 0.7);
            }
            return;
        }
        let dot = |p: &mut Painter, at: Point| {
            p.shape(ShapeInstance::circle(
                at,
                6.0 * k,
                Paint::glow(hex(CYAN, 0.9), 9.0 * k, 1.2),
            ));
            p.shape(ShapeInstance::circle(at, 2.4 * k, Paint::solid(hex(BRIGHT, 1.0))));
        };
        let lerp = |a: Point, b: Point, f: f32| {
            let f = 1.0 - (1.0 - f) * (1.0 - f);
            Point::new(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f)
        };
        let (start, end) = (Point::new(l.x0, l.y), Point::new(l.x1, l.y));
        match l.trip(t) {
            Trip::Home => {}
            Trip::Out(f) => {
                if let Some(h) = self.head_at(l.from, edge) {
                    dot(p, lerp(h, start, f));
                }
            }
            Trip::Draw(f) => {
                line(p, f, 0.95);
                dot(p, Point::new(l.x0 + (l.x1 - l.x0) * f, l.y));
            }
            Trip::Hold => {
                line(p, 1.0, 0.95);
                dot(p, end);
            }
            Trip::Back(f) => {
                line(p, 1.0, 0.95 * (1.0 - f));
                if let Some(h) = self.head_at(t, edge) {
                    dot(p, lerp(end, h, f));
                }
            }
        }
    }

    /// A bright dot with a glow at `s` along the border.
    fn head(&self, p: &mut Painter, edge: &Contour, s: f32) {
        if let Some((at, _)) = edge.at(s) {
            p.shape(ShapeInstance::circle(
                at,
                6.0 * self.k,
                Paint::glow(hex(CYAN, 0.9), 9.0 * self.k, 1.2),
            ));
            p.shape(ShapeInstance::circle(
                at,
                2.4 * self.k,
                Paint::solid(hex(BRIGHT, 1.0)),
            ));
        }
    }
}

/// The transcript's type: Saira Light, scaled with the popup.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = look.width / CONCEPT_W;
    TextStyle::new(fonts(look).text, TEXT_PX * k, LINE * k)
}

/// Width the transcript wraps at.
pub(crate) fn text_width(look: &Look) -> f32 {
    let k = look.width / CONCEPT_W;
    634.0 * k
}

impl Popup {
    pub(crate) fn tron_surface(look: &Look) -> Size {
        Geo::new(look).surface()
    }

    /// Notes when a dictation ended without a final text.
    pub(crate) fn sync_tron(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Tron {
            return;
        }
        if self.ended() && self.extra.final_at.is_none() && self.tron.ended_at.is_none() {
            self.tron.ended_at = Some(t);
        }
    }

    pub(crate) fn tron_view(&self) -> Element<Msg> {
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
        let relax_at = self
            .extra
            .finish_at
            .or(self.tron.ended_at)
            .or_else(|| finishing.then_some(self.extra.now));
        let lines = Lines {
            k,
            bottom,
            shown_at: self.shown_at,
            relax_at,
            final_at: self.extra.final_at,
            ended: self.tron.ended_at.is_some(),
            progress: self
                .shows_progress(&status)
                .then(|| self.files.fraction.map(|f| f as f32))
                .flatten(),
            loading: status.loading,
            wave: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
            scrub: self.scrub(),
            light: self.tron_light(k),
        };
        let until = match (self.extra.final_at, self.tron.ended_at) {
            (Some(f), _) => f + FLARE_SECS + 0.1,
            _ => f64::INFINITY,
        };
        let canvas = at(
            Element::canvas("lines", until, move |t, r, p| {
                p.layer(vo::scene::geom::Affine::translate(r.x, r.y), 1.0, |p| {
                    lines.draw(t, p)
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
        let panel = at(Element::layer("tron"), MARGIN, MARGIN)
            .size(g.w, bottom * k)
            .bloom(0.12, 5.0 * k)
            .presence(Transition::fade().opacity(1.0), exit)
            .child(canvas)
            .child(self.tron_badge(&g, &status))
            .children(self.tron_lattice(&g))
            .child(self.tron_text(&g))
            .children(self.tron_rewind(&g))
            .children(self.joint_ticks_element(
                "tron",
                &self.shown,
                &self.look.text_style(),
                TEXT_X * g.k,
                TEXT_TOP * g.k,
                self.look.text_width(),
                hex(CYAN, 1.0),
                true,
            ))
            .child(self.tron_rule(&g))
            .child(self.tron_status(&g, &status, bottom));
        root.child(panel)
    }

    /// MO-68: the light line for the newest word, in the canvas's pixels.
    fn tron_light(&self, k: f32) -> Option<LightLine> {
        if !self.look.on(LIGHT_LINE) {
            return None;
        }
        let newest = self.shown.last()?;
        let heard = self.heard.word(newest.id)?;
        let style = self.look.text_style();
        let (boxes, _) =
            sounding::word_boxes(&self.measure, &self.shown, &style, Some(self.look.text_width()));
        let b = boxes.iter().find(|b| b.id == newest.id)?;
        let from = heard.onset.unwrap_or(heard.born);
        Some(LightLine {
            from,
            landed: heard.born.max(from + LIGHT_LEAVE),
            x0: TEXT_X * k + b.rect.x,
            x1: TEXT_X * k + b.rect.x + b.rect.w,
            y: TEXT_TOP * k + b.baseline + 5.0 * k,
            reduced: self.look.on("reduced-motion"),
        })
    }

    /// The microphone, the stop square, the check or the cross in the badge.
    fn tron_badge(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let side = 44.0 * k;
        let place = |e: Element<Msg>, s: f32| {
            at(e.size(s, s), BADGE_X * k - s / 2.0, BADGE_Y * k - s / 2.0)
        };
        let slot = at(Element::layer("badge"), 0.0, 0.0);
        let mark = |key: &'static str, o: &Arc<Outline>| {
            Element::stroke_path(
                key,
                o.clone(),
                view,
                StrokeStyle::solid(3.0 * k, hex(BRIGHT, 1.0)).glow(5.0 * k),
            )
            .motion(Prop::Trim, Spring::critical_to_98(0.4))
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
        };
        if self.extra.final_at.is_some() {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot.child(place(mark("check", check), side));
        }
        if self.tron.ended_at.is_some() {
            let cross = CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)));
            return slot.child(place(mark("cross", cross), side * 0.8));
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
                StrokeStyle::solid(2.0 * k, hex(BRIGHT, 1.0)).glow(2.0 * k),
            )
            .morph_to(if stopped { 1.0 } else { 0.0 })
            .motion(Prop::Morph, Spring::critical_to_98(0.3))
            .presence(Transition::fade(), Transition::fade().scale(0.6)),
            if stopped { side * 0.7 } else { side },
        ))
    }

    /// LT-07: with `kinetic-lattice`, the lattice in the light style under
    /// the words, its newest gutter the gap down to the rule.
    fn tron_lattice(&self, g: &Geo) -> Option<Element<Msg>> {
        if !self.look.on(wild_space::KINETIC_LATTICE) {
            return None;
        }
        let k = g.k;
        let set = wild_space::LatticeSet {
            preset: vo::scene::lattice::LatticeStyle::Light,
            color: hex(DETAIL, 0.5),
            accent: hex(BRIGHT, 1.0),
            scale: 0.9 * k,
            label: Some(TextStyle::new(fonts(&self.look).status, 9.0 * k, 11.0 * k)),
            gutter: (RULE_GAP - 4.0) * k,
            opacity: if self.tron.ended_at.is_some() { 0.35 } else { 1.0 },
        };
        let style = self.look.text_style();
        let e = self.lattice_lines("tron", &self.shown, &style, self.look.text_width(), &set)?;
        Some(at(e, TEXT_X * k, TEXT_TOP * k))
    }

    /// The transcript: words fade up out of a blur.
    fn tron_text(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let mut words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .text_style(self.look.text_style())
            .color(hex(TEXT, 1.0))
            .text_halo(hex(CYAN, 0.18), 4.0 * k)
            .width(self.look.text_width())
            .motion(Prop::Opacity, Spring::critical_to_98(0.18))
            .motion(Prop::Offset, Spring::critical_to_98(0.22))
            .motion(Prop::TextFx, Spring::critical_to_98(0.22))
            .word_enter(Transition::fade().offset(0.0, 5.0 * k).blur(3.0 * k));
        if self.tron.ended_at.is_some() {
            words = words.opacity(0.35);
        }
        let words = self.handoff_or(
            words,
            &self.look.text_style(),
            Some(self.look.text_width()),
            hex(TEXT, 1.0),
            hex(CYAN, 1.0),
        );
        at(words.rewind_hide(self.rewinding()), TEXT_X * k, TEXT_TOP * k)
    }

    /// MO-50: a cancel takes the words back into the caret, drawn over
    /// the hidden text.
    fn tron_rewind(&self, g: &Geo) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let ink = endings::Ink {
            text: hex(TEXT, 0.35),
            mark: hex(CYAN, 1.0),
            faint: hex(DIM, 1.0),
        };
        let (style, w) = (self.look.text_style(), self.look.text_width());
        let h = self.lines.max(1) as f32 * style.line_height;
        let laid = self.lay_words(style, w);
        self.rewind_mark("rewind", laid, ink, Size::new(w, h))
            .map(|e| at(e, TEXT_X * g.k, TEXT_TOP * g.k))
    }

    /// The hairline over the status line, opening from its middle.
    fn tron_rule(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let y = (TEXT_TOP + self.lines as f32 * LINE + RULE_GAP) * k;
        at(Element::shape("rule", ShapeKind::RRect), TEXT_X * k, y)
            .size(g.w - 2.0 * TEXT_X * k, 1.2 * k)
            .fill(hex(DIM, 0.95))
            .motion(Prop::Position, Spring::critical_to_98(0.25))
            .presence(Transition::fade().scale(0.0), Transition::fade())
    }

    /// The centred status line: a spinner ring, the state and the detail.
    fn tron_status(&self, g: &Geo, status: &StatusLine, bottom: f32) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
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
        } else if self.tron.ended_at.is_some() {
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
        let style = TextStyle::new(f.status, STATUS_PX * k, 24.0 * k);
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!("·  {detail}")
        };
        let wid = |s: &str| self.measure.layout(s, &style, None).size.w.ceil() + 2.0;
        let icon_side = 20.0 * k;
        let shown_at = self.shown_at;
        let icon = if done {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            Element::layer("icon")
                .size(icon_side, icon_side)
                .child(
                    at(Element::shape("ring", ShapeKind::Circle), 0.0, 0.0)
                        .size(icon_side, icon_side)
                        .fill(Color::TRANSPARENT)
                        .border(hex(CYAN, 1.0), 1.6 * k),
                )
                .child(at(
                    Element::stroke_path(
                        "check",
                        check.clone(),
                        Rect::new(0.0, 0.0, 24.0, 24.0),
                        StrokeStyle::solid(2.0 * k, hex(CYAN, 1.0)),
                    )
                    .size(icon_side * 0.6, icon_side * 0.6)
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    icon_side * 0.2,
                    icon_side * 0.2,
                ))
        } else {
            let still = self.tron.ended_at.is_some();
            Element::canvas(
                "spinner",
                if still { 0.0 } else { f64::INFINITY },
                move |t, r, p| {
                    let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
                    let rad = r.w * 0.38;
                    p.stroke(
                        &make::ring(c, rad),
                        &StrokeStyle::solid(1.8 * k, hex(DIM, 0.9)),
                    );
                    p.stroke(
                        &make::arc(c, rad, (t - shown_at) as f32 * 5.0, 4.4),
                        &StrokeStyle::solid(1.9 * k, hex(CYAN, 1.0)),
                    );
                },
            )
            .size(icon_side, icon_side)
        };
        let row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .gap(9.0 * k)
            .child(icon)
            .child(
                Element::label(format!("state-{state}"), state.as_str())
                    .text_style(style)
                    .color(hex(CYAN, 1.0))
                    .width(wid(&state))
                    .presence(Transition::fade().offset(0.0, 6.0 * k), Transition::fade()),
            )
            .child(
                Element::label(format!("detail-{state}"), detail.as_str())
                    .text_style(style)
                    .color(hex(DETAIL, 1.0))
                    .width(wid(&detail))
                    .presence(Transition::fade().offset(0.0, 6.0 * k), Transition::fade()),
            );
        let y = (bottom - BOTTOM_GAP - STEP) * k - 12.0 * k;
        at(Element::column("status-box"), 0.0, y)
            .width(g.w)
            .align_items(AlignItems::CENTER)
            .child(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_border_starts_at_the_badge_and_ends_back_at_it() {
        let b = border(500.0);
        let pts = b.resample(4);
        let first = pts.first().unwrap();
        let last = pts.last().unwrap();
        assert!(first.x > BADGE_X && last.x < BADGE_X);
        assert!((first.y - last.y).abs() < 1e-3);
    }

    #[test]
    fn the_line_is_flat_at_its_ends_and_follows_the_level_in_the_middle() {
        let lines = Lines {
            k: 1.0,
            bottom: 500.0,
            shown_at: 0.0,
            relax_at: None,
            final_at: None,
            ended: false,
            progress: None,
            loading: false,
            wave: Arc::from([1.0f32; 200].as_slice()),
            newest_at: 1.0,
            rate: 100.0,
            scrub: endings::Scrub::default(),
            light: None,
        };
        assert_eq!(lines.wave_y(1.0, 0.0, 0.0, 5.0, 1.0), 0.0);
        let peak = (0..50)
            .map(|i| {
                lines
                    .wave_y(1.0, 0.4 + i as f32 * 0.004, 0.0, 5.0, 1.0)
                    .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(peak > WAVE_AMP * 0.3);
    }

    #[test]
    fn the_light_head_flies_out_draws_the_line_and_comes_back() {
        let l = LightLine {
            from: 1.0,
            landed: 1.3,
            x0: 10.0,
            x1: 60.0,
            y: 100.0,
            reduced: false,
        };
        assert_eq!(l.trip(0.9), Trip::Home);
        assert!(matches!(l.trip(1.15), Trip::Out(f) if (f - 0.5).abs() < 1e-3));
        assert!(matches!(l.trip(1.3 + LIGHT_DRAW / 2.0), Trip::Draw(f) if (f - 0.5).abs() < 1e-3));
        assert_eq!(l.trip(1.3 + LIGHT_DRAW + 0.1), Trip::Hold);
        let back = 1.3 + LIGHT_DRAW + LIGHT_HOLD;
        assert!(matches!(l.trip(back + 0.01), Trip::Back(_)));
        assert_eq!(l.trip(back + LIGHT_BACK + 0.01), Trip::Home);
        assert!(l.away(1.2) && !l.away(3.0));
        // Reduced motion: the head never leaves the border.
        assert!(!LightLine { reduced: true, ..l }.away(1.2));
    }
}
