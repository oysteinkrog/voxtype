//! The `amber` look of the vo popup: an amber hologram panel. Thin chamfered
//! frames with corner brackets, vertical micro-labels down both sides, a
//! fine-line waveform of many thin bars and the transcript in a condensed
//! face, over dark smoked glass with faint scanlines.
//!
//! The frame draws itself on when the popup opens. While listening the
//! waveform is rebuilt from the level history every frame: the newest level
//! in the middle, older ones toward both ends. Once the recording stops the
//! bars sink to a dotted line and the dots light up from the left with the
//! final pass's progress. When the text lands the dots join into one solid
//! line and a check takes the place of the level light.
//!
//! Geometry is laid out in the pixels of the concept image and scaled so the
//! image is as wide as the popup.

use super::*;

use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::Point;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Width of the concept image, in concept pixels.
const CONCEPT_W: f32 = 725.0;
/// Room round the popup for the glow, in logical pixels.
const MARGIN: f32 = 10.0;

// Layout in concept pixels.
const FRAME: f32 = 12.0;
const CHAMFER: f32 = 16.0;
const MIC_X: f32 = 52.0;
const MIC_Y: f32 = 44.0;
const TITLE_X: f32 = 80.0;
const TITLE_Y: f32 = 30.0;
const TITLE_PX: f32 = 11.5;
const WAVE_Y: f32 = 104.0;
const WAVE_X0: f32 = 55.0;
const WAVE_X1: f32 = 645.0;
const WAVE_AMP: f32 = 52.0;
const WAVE_BARS: usize = 91;
const RULE_Y: f32 = 160.0;
const TEXT_X: f32 = 95.0;
const TEXT_TOP: f32 = 180.0;
const LINE: f32 = 50.0;
const TEXT_PX: f32 = 38.5;
const TRACKING: f32 = 1.2;
const RULE2_GAP: f32 = 34.0;
const STATUS_GAP: f32 = 33.0;
const BOTTOM_GAP: f32 = 42.0;
const STATUS_PX: f32 = 17.0;
const JP_PX: f32 = 15.0;
const JP_STEP: f32 = 18.5;
/// The labels down the sides.
const LEFT_LABEL: &str = "音声をテキストに変換中";
const RIGHT_LABEL: &str = "音声入力";

/// Seconds for the frame to draw itself on.
const OPEN_SECS: f64 = 0.6;
/// Seconds of level history from the middle of the waveform to each end.
const HISTORY_SECS: f32 = 1.4;
/// Seconds for the bars to sink to dots once the recording stops.
const SINK_SECS: f64 = 0.45;
/// Seconds for the dots to join into a line when the text lands.
const JOIN_SECS: f64 = 0.5;

const AMBER: u32 = 0xff9a2e;
const HOT: u32 = 0xffd59a;
const DEEP: u32 = 0xa8520f;
const GLASS: u32 = 0x120803;
const TEXT: u32 = 0xffb24f;
const SOFT: u32 = 0xf2c99a;

fn hex(rgb: u32, a: f32) -> Color {
    Color::hex(rgb, a)
}

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

/// A fixed pseudo-random number in 0..1 for `i`.
fn seed(i: usize) -> f32 {
    let x = (i as u32).wrapping_mul(0x9e37_79b9).rotate_left(11) ^ 0x7f4a_7c15;
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

/// Fonts for the transcript, the small caps labels and the side labels.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    small: u32,
    jp: u32,
}

/// Oswald Light, the closest free match to the concept's condensed face,
/// Barlow for the small labels and IBM Plex Sans JP for the side labels
/// (2.4 MB, where Noto Sans CJK is 19 MB; any Japanese face when missing),
/// or the look's own sans when one is not installed.
fn fonts(look: &Look) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |family: &str| look.fonts.add_system(family).unwrap_or(SANS);
        Fonts {
            text: add("Oswald:light"),
            small: add("Barlow:regular"),
            jp: add("IBM Plex Sans JP:regular:lang=ja"),
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

    /// The rule under the transcript, in concept pixels.
    fn rule2(lines: u32) -> f32 {
        TEXT_TOP + lines as f32 * LINE + RULE2_GAP
    }

    /// Bottom of the outer frame, in concept pixels.
    fn bottom(lines: u32) -> f32 {
        Geo::rule2(lines) + STATUS_GAP + BOTTOM_GAP
    }

    fn surface(&self) -> Size {
        let h = (Geo::bottom(swiss::MAX_LINES as u32) + FRAME) * self.k + 2.0 * MARGIN;
        Size::new((self.w + 2.0 * MARGIN).ceil(), h.ceil())
    }
}

/// The outer frame as one closed contour in concept pixels, from the top
/// left running clockwise, with a step down in the top edge on the right.
fn frame(bottom: f32) -> Contour {
    let (l, r, t, c) = (FRAME, CONCEPT_W - FRAME, FRAME, CHAMFER);
    let pts = [
        Point::new(l + c, t),
        Point::new(488.0, t),
        Point::new(498.0, t + 10.0),
        Point::new(r - c, t + 10.0),
        Point::new(r, t + 10.0 + c),
        Point::new(r, bottom - c),
        Point::new(r - c, bottom),
        Point::new(l + c, bottom),
        Point::new(l, bottom - c),
        Point::new(l, t + c),
    ];
    Contour::polyline(&pts, true)
}

/// State only the amber look keeps.
#[derive(Default)]
pub struct State {
    /// When the dictation ended without a final text.
    ended_at: Option<f64>,
}

/// What the canvas draws, fixed for one view.
struct Panel {
    k: f32,
    lines: u32,
    shown_at: f64,
    /// When the bars start to sink.
    sink_at: Option<f64>,
    final_at: Option<f64>,
    ended: bool,
    progress: Option<f32>,
    loading: bool,
    wave: Arc<[f32]>,
    newest_at: f64,
    rate: f32,
}

impl Panel {
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

    /// Half the height of bar `i` while listening, in concept pixels: the
    /// newest level in the middle, older ones toward the ends.
    fn bar(&self, t: f64, i: usize) -> f32 {
        let x = i as f32 / (WAVE_BARS - 1) as f32;
        let d = (x - 0.5).abs() * 2.0;
        let window = (1.0 - d * d).max(0.0).powf(1.2);
        let level = (self.level_at(t, d * HISTORY_SECS) * 4.0).min(1.0);
        let s = seed(i);
        let ripple = 0.55 + 0.45 * (TAU * (x * 9.0 + s * 0.3) - t as f32 * 3.0).sin().abs();
        WAVE_AMP * window * level * (0.35 + 0.65 * s) * ripple
    }

    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let open = ease_out((t - self.shown_at) / OPEN_SECS);
        let sink = self
            .sink_at
            .map_or(0.0, |s| ease_out((t - s) / SINK_SECS));
        let join = self.final_at.map_or(0.0, |f| ease_out((t - f) / JOIN_SECS));
        let fade = if self.ended { 0.45 } else { 1.0 };
        let bottom = Geo::bottom(self.lines);
        let line = |w: f32, c: u32, a: f32| StrokeStyle::solid(w * k, hex(c, a * fade));

        // The smoked glass and the outer frame, drawn on from the top left.
        let edge = frame(bottom).transform(&vo::scene::geom::Affine::scale(k));
        p.fill([&edge], &StrokeStyle::solid(1.0, hex(GLASS, 0.88 * open)));
        let len = edge.length();
        p.stroke(
            &edge.trim(0.0, len * open),
            &line(1.6, AMBER, 0.95).glow(4.0 * k),
        );

        // Corner brackets just inside the frame.
        let br = 22.0;
        let inset = FRAME + 7.0;
        let r = CONCEPT_W - inset;
        let b = bottom - 7.0;
        for (x, y, dx, dy) in [
            (inset, inset + 8.0, 1.0, 1.0),
            (r, inset + 18.0, -1.0, 1.0),
            (inset, b, 1.0, -1.0),
            (r, b, -1.0, -1.0),
        ] {
            let c = Contour::polyline(
                &[
                    self.p(x, y + dy * br),
                    self.p(x, y),
                    self.p(x + dx * br, y),
                ],
                false,
            );
            p.stroke(&c, &line(2.2, HOT, 0.9 * open).glow(3.0 * k));
        }

        // The header's underline and the two rules.
        let h = |x0: f32, x1: f32, y: f32, a: f32| {
            ShapeInstance::line(
                self.p(x0, y),
                self.p(x0 + (x1 - x0) * open, y),
                1.0 * k,
                Paint::solid(hex(AMBER, a * fade)),
            )
        };
        p.shape(h(75.0, 215.0, 53.0, 0.55));
        p.shape(h(30.0, CONCEPT_W - 30.0, RULE_Y, 0.6));
        p.shape(h(30.0, CONCEPT_W - 30.0, Geo::rule2(self.lines), 0.6));
        // Ticks at the ends of the first rule.
        for x in [30.0, CONCEPT_W - 30.0] {
            p.shape(ShapeInstance::line(
                self.p(x, RULE_Y - 4.0),
                self.p(x, RULE_Y + 4.0),
                1.0 * k,
                Paint::solid(hex(AMBER, 0.6 * fade * open)),
            ));
        }

        // The line between the left label and the transcript.
        let lb = Geo::rule2(self.lines) - 14.0;
        p.shape(ShapeInstance::line(
            self.p(70.0, 178.0),
            self.p(70.0, lb - 6.0),
            1.2 * k,
            Paint::solid(hex(AMBER, 0.7 * fade * open)),
        ));

        // The scroll mark on the right: a dotted track.
        for i in 0..14 {
            let y = 192.0 + i as f32 * 5.5;
            p.shape(ShapeInstance::line(
                self.p(690.0, y),
                self.p(690.0, y + 3.0),
                2.0 * k,
                Paint::solid(hex(AMBER, (if i < 6 { 0.85 } else { 0.35 }) * fade * open)),
            ));
        }

        // The slanted marks bottom right.
        let sy = Geo::rule2(self.lines) + STATUS_GAP;
        for row in 0..2 {
            for i in 0..5 {
                let x = 645.0 + i as f32 * 8.0 + row as f32 * 3.0;
                let y = sy - 5.0 + row as f32 * 7.0;
                p.shape(ShapeInstance::line(
                    self.p(x, y + 4.0),
                    self.p(x + 4.0, y),
                    2.2 * k,
                    Paint::solid(hex(AMBER, 0.8 * fade * open)),
                ));
            }
        }

        // The waveform: thin bars, sinking to dots after the stop and
        // joining into one line when the text lands.
        let step = (WAVE_X1 - WAVE_X0) / (WAVE_BARS - 1) as f32;
        let lit = self.progress.unwrap_or(0.0).clamp(0.0, 1.0);
        let scan = ((t - self.shown_at) * 0.6).rem_euclid(1.0) as f32;
        for i in 0..WAVE_BARS {
            let x = WAVE_X0 + i as f32 * step;
            let u = i as f32 / (WAVE_BARS - 1) as f32;
            // Bars appear from the middle outward as the panel opens.
            if (u - 0.5).abs() * 2.0 > open {
                continue;
            }
            let live = self.bar(t, i);
            let half = (live * (1.0 - sink)).max(1.2);
            let tall = (live / WAVE_AMP).min(1.0) * (1.0 - sink);
            let mut a = 0.55 + 0.45 * tall;
            if self.loading && self.sink_at.is_none() {
                let d = (u - scan).rem_euclid(1.0);
                a = 0.3 + 0.7 * (1.0 - d).powi(8);
            } else if sink > 0.0 && self.final_at.is_none() {
                a = if u <= lit { 1.0 } else { 0.4 };
            }
            let col = if tall > 0.55 { HOT } else { AMBER };
            p.shape(ShapeInstance::line(
                self.p(x, WAVE_Y - half),
                self.p(x, WAVE_Y + half),
                (1.6 + 0.6 * tall) * k,
                Paint::solid(hex(col, a * fade)),
            ));
        }
        if join > 0.0 {
            // The line grows from the middle out to both ends.
            let mid = (WAVE_X0 + WAVE_X1) / 2.0;
            let half = (WAVE_X1 - WAVE_X0) / 2.0 * join;
            p.stroke(
                &Contour::polyline(
                    &[self.p(mid - half, WAVE_Y), self.p(mid + half, WAVE_Y)],
                    false,
                ),
                &line(1.2, AMBER, 0.75).glow(3.0 * k),
            );
        }
    }
}

/// The transcript's type: Oswald Light, scaled with the popup.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = look.width / CONCEPT_W;
    TextStyle::new(fonts(look).text, TEXT_PX * k, LINE * k).tracking(TRACKING * k)
}

/// Width the transcript wraps at.
pub(crate) fn text_width(look: &Look) -> f32 {
    let k = look.width / CONCEPT_W;
    585.0 * k
}

impl Popup {
    pub(crate) fn amber_surface(look: &Look) -> Size {
        Geo::new(look).surface()
    }

    /// Notes when a dictation ended without a final text.
    pub(crate) fn sync_amber(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Amber {
            return;
        }
        if self.ended() && self.extra.final_at.is_none() && self.amber.ended_at.is_none() {
            self.amber.ended_at = Some(t);
        }
    }

    pub(crate) fn amber_view(&self) -> Element<Msg> {
        let g = Geo::new(&self.look);
        let size = g.surface();
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        let k = g.k;
        let status = self.status();
        let lines = self.lines;
        let bottom = Geo::bottom(lines) + FRAME;
        let done = self.extra.final_at.is_some();
        let finishing = status.finishing && !status.loading;
        let sink_at = self
            .extra
            .finish_at
            .or(self.amber.ended_at)
            .or_else(|| finishing.then_some(self.extra.now));
        let panel = Panel {
            k,
            lines,
            shown_at: self.shown_at,
            sink_at,
            final_at: self.extra.final_at,
            ended: self.amber.ended_at.is_some(),
            progress: self
                .shows_progress(&status)
                .then(|| self.files.fraction.map(|f| f as f32))
                .flatten(),
            loading: status.loading,
            wave: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
        };
        let until = match (self.extra.final_at, self.amber.ended_at) {
            (Some(f), _) => f + JOIN_SECS + 0.1,
            (None, Some(e)) => e + SINK_SECS + 0.1,
            _ => f64::INFINITY,
        };
        let canvas = at(
            Element::canvas("panel", until, move |t, r, p| {
                p.layer(vo::scene::geom::Affine::translate(r.x, r.y), 1.0, |p| {
                    panel.draw(t, p)
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
        let layer = at(Element::layer("amber"), MARGIN, MARGIN)
            .size(g.w, bottom * k)
            .scanlines(0.12, 3.0 * k)
            .flicker(0.02)
            .bloom(0.22, 6.0 * k)
            .presence(Transition::fade().opacity(1.0), exit)
            .child(canvas)
            .child(self.amber_header(&g, &status))
            .child(self.amber_side_labels(&g))
            .child(self.amber_text(&g))
            .child(self.amber_status(&g, &status));
        root.child(layer)
    }

    /// The microphone and the spaced-out title.
    fn amber_header(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static MIC: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let f = fonts(&self.look);
        let mic = MIC.get_or_init(|| Arc::new(Outline::new(&icons::mic(), 0.05)));
        let side = 38.0 * k;
        let finishing = status.finishing && !status.loading && self.extra.final_at.is_none();
        let title = if finishing && self.amber.ended_at.is_none() {
            "VOICE DICTATION  ·  FINAL PASS"
        } else {
            "VOICE DICTATION"
        };
        let style = TextStyle::new(f.small, TITLE_PX * k, 16.0 * k).tracking(3.0 * k);
        let w = self.measure.layout(title, &style, None).size.w.ceil() + 4.0;
        Element::layer("header")
            .child(at(
                Element::stroke_path(
                    "mic",
                    mic.clone(),
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                    StrokeStyle::solid(1.6 * k, hex(AMBER, 1.0)),
                )
                .size(side, side),
                MIC_X * k - side / 2.0,
                MIC_Y * k - side / 2.0,
            ))
            .child(at(
                Element::label(format!("title-{title}"), title)
                    .text_style(style)
                    .color(hex(SOFT, 0.9))
                    .width(w)
                    .presence(Transition::fade().wipe(1.0), Transition::fade()),
                TITLE_X * k,
                TITLE_Y * k,
            ))
    }

    /// The vertical labels down the left and the right, one character a
    /// row.
    fn amber_side_labels(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let f = fonts(&self.look);
        let style = TextStyle::new(f.jp, JP_PX * k, JP_STEP * k);
        let column = |key: &str, text: &str, x: f32, y: f32, end: f32, a: f32| {
            let mut e = Element::layer(key.to_string());
            // A short popup shows only the characters that fit above `end`.
            let fit = ((end - y) / JP_STEP).floor().max(0.0) as usize;
            for (i, ch) in text.chars().take(fit).enumerate() {
                let s = ch.to_string();
                e = e.child(at(
                    Element::label(format!("{key}-{i}"), s.as_str())
                        .text_style(style)
                        .color(hex(AMBER, a))
                        .width(JP_PX * k * 1.2)
                        .presence(Transition::fade().wipe(1.0), Transition::fade()),
                    (x - JP_PX / 2.0) * k,
                    (y + i as f32 * JP_STEP) * k,
                ));
            }
            e
        };
        Element::layer("labels")
            .child(column(
                "jp-left",
                LEFT_LABEL,
                47.0,
                176.0,
                Geo::rule2(self.lines) - 4.0,
                0.85,
            ))
            .child(column("jp-right", RIGHT_LABEL, 682.0, 42.0, RULE_Y, 0.85))
    }

    /// The transcript: words wipe in from the left like a projector
    /// catching them.
    fn amber_text(&self, g: &Geo) -> Element<Msg> {
        let k = g.k;
        let mut words = Element::live_text("words", &self.shown)
            .text_style(self.look.text_style())
            .color(hex(TEXT, 1.0))
            .text_halo(hex(AMBER, 0.25), 5.0 * k)
            .width(self.look.text_width())
            .motion(Prop::Opacity, Spring::critical_to_98(0.22))
            .motion(Prop::Offset, Spring::critical_to_98(0.22))
            .word_enter(Transition::fade().wipe(1.0))
            .word_change(Transition::fade().opacity(0.3).offset(0.0, 4.0 * k));
        if self.amber.ended_at.is_some() {
            words = words.opacity(0.4);
        }
        let words = self.handoff_or(
            words,
            &self.look.text_style(),
            Some(self.look.text_width()),
            hex(TEXT, 1.0),
            hex(AMBER, 1.0),
        );
        at(words, TEXT_X * k, TEXT_TOP * k)
    }

    /// The status line: a level light, a spinner, a check or a cross, then
    /// the state and the detail.
    fn amber_status(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let k = g.k;
        let f = fonts(&self.look);
        let done = self.extra.final_at.is_some();
        let cancelled = self.amber.ended_at.is_some();
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
        let style = TextStyle::new(f.small, STATUS_PX * k, 20.0 * k);
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!("·    {detail}")
        };
        let wid = |s: &str| self.measure.layout(s, &style, None).size.w.ceil() + 2.0;
        let side = 32.0 * k;
        let icon = if done || cancelled {
            let o = if done {
                CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)))
            } else {
                CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)))
            };
            let s = side * 0.6;
            Element::layer("icon")
                .size(side, side)
                .child(
                    at(Element::shape("ring", ShapeKind::Circle), 0.0, 0.0)
                        .size(side, side)
                        .fill(Color::TRANSPARENT)
                        .border(hex(AMBER, 1.0), 1.6 * k),
                )
                .child(at(
                    Element::stroke_path(
                        if done { "check" } else { "cross" },
                        o.clone(),
                        Rect::new(0.0, 0.0, 24.0, 24.0),
                        StrokeStyle::solid(2.2 * k, hex(HOT, 1.0)),
                    )
                    .size(s, s)
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    (side - s) / 2.0,
                    (side - s) / 2.0,
                ))
        } else {
            let spin = status.finishing || status.loading;
            let wave = self.wave.clone();
            let newest_at = self.newest_at;
            let shown_at = self.shown_at;
            Element::canvas("light", f64::INFINITY, move |t, r, p| {
                let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
                let rad = r.w * 0.44;
                if spin {
                    p.stroke(
                        &make::ring(c, rad),
                        &StrokeStyle::solid(1.6 * k, hex(DEEP, 0.9)),
                    );
                    p.stroke(
                        &make::arc(c, rad, (t - shown_at) as f32 * 4.5, 2.2),
                        &StrokeStyle::solid(2.2 * k, hex(AMBER, 1.0)).glow(3.0 * k),
                    );
                    return;
                }
                let n = wave.len();
                let lvl = if n == 0 {
                    0.0
                } else {
                    let back = ((newest_at - t) * FRAME_HZ as f64).max(0.0) as usize;
                    wave[n - 1 - back.min(n - 1)]
                };
                let lvl = (lvl * 4.0).min(1.0);
                p.stroke(
                    &make::ring(c, rad),
                    &StrokeStyle::solid(1.8 * k, hex(AMBER, 0.95)).glow(2.0 * k),
                );
                let breathe = 0.5 + 0.5 * ((t - shown_at) as f32 * PI).sin();
                p.shape(ShapeInstance::circle(
                    c,
                    rad * (0.45 + 0.2 * lvl),
                    Paint::glow(hex(AMBER, 0.8), 6.0 * k, 0.6 + 0.6 * lvl + 0.2 * breathe),
                ));
                p.shape(ShapeInstance::circle(
                    c,
                    rad * (0.42 + 0.18 * lvl),
                    Paint::solid(hex(HOT, 1.0)),
                ));
            })
            .size(side, side)
        };
        let row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .gap(16.0 * k)
            .child(icon)
            .child(
                Element::label(format!("state-{state}"), state.as_str())
                    .text_style(style)
                    .color(hex(SOFT, 1.0))
                    .width(wid(&state))
                    .presence(Transition::fade().wipe(1.0), Transition::fade()),
            )
            .child(
                Element::label(format!("detail-{state}"), detail.as_str())
                    .text_style(style)
                    .color(hex(SOFT, 0.85))
                    .width(wid(&detail))
                    .presence(Transition::fade().wipe(1.0), Transition::fade()),
            );
        let y = (Geo::rule2(self.lines) + STATUS_GAP) * k - side / 2.0;
        at(row, 72.0 * k, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(level: f32) -> Panel {
        Panel {
            k: 1.0,
            lines: 3,
            shown_at: 0.0,
            sink_at: None,
            final_at: None,
            ended: false,
            progress: None,
            loading: false,
            wave: Arc::from(vec![level; 300].as_slice()),
            newest_at: 2.0,
            rate: 100.0,
        }
    }

    #[test]
    fn the_end_bars_stay_flat_and_the_middle_follows_the_level() {
        let loud = panel(0.5);
        assert_eq!(loud.bar(2.0, 0), 0.0);
        assert_eq!(loud.bar(2.0, WAVE_BARS - 1), 0.0);
        let peak = (35..56).map(|i| loud.bar(2.0, i)).fold(0.0f32, f32::max);
        assert!(peak > WAVE_AMP * 0.3);
        let quiet = panel(0.0);
        assert_eq!(quiet.bar(2.0, WAVE_BARS / 2), 0.0);
    }

    #[test]
    fn the_frame_is_closed_and_inside_the_image() {
        let f = frame(430.0);
        assert!(f.closed);
        for p in f.resample(8) {
            assert!(p.x >= FRAME - 1e-3 && p.x <= CONCEPT_W - FRAME + 1e-3);
            assert!(p.y >= FRAME - 1e-3 && p.y <= 430.0 + 1e-3);
        }
    }
}
