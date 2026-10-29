//! The `palette` look of the vo popup, after mock-up 17: a command palette
//! in the manner of Raycast or Linear. A crisp dark card with chips for the
//! engine, how the text goes out and the latency, a blue to violet bar
//! waveform, the transcript as the input text with a violet caret, words the
//! engine is unsure of softly underlined, and a key badge in the status.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::kit::{at, ease_out, facts, hex, mix, reduced, Levels, Mode};
use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;

/// Colours sampled from the mock-up.
const CARD: u32 = 0x070D1A;
const EDGE: u32 = 0x34486A;
const CHIP: u32 = 0x101A2F;
const CHIP_EDGE: u32 = 0x26324A;
const INK: u32 = 0xE6EAF0;
const SOFT: u32 = 0x9AA3B2;
const BLUE: u32 = 0x35A4FD;
const VIOLET: u32 = 0x7246FC;
const LILAC: u32 = 0xC0ADF6;
const TEAL: u32 = 0x35D2CD;
const RULE: u32 = 0x1C2741;
const BADGE: u32 = 0x1A2236;
const DOUBT: u32 = 0x6EA8FF;
const GREY: u32 = 0x6B7486;

const DESIGN_W: f32 = 600.0;
const RADIUS: f32 = 12.5;
const PAD_TOP: f32 = 22.0;
const CHIP_H: f32 = 30.0;
/// The waveform box and the bars in it.
const WAVE_GAP: f32 = 26.0;
const WAVE_H: f32 = 68.0;
const WAVE_HALF: f32 = 31.0;
const BAR_X0: f32 = 31.0;
const BAR_X1: f32 = 570.0;
const BAR_PITCH: f32 = 9.4;
const BAR_W: f32 = 3.4;
const BAR_SECS: f64 = 0.035;
/// The transcript: Inter Regular, as the input text of the palette.
const TEXT_X: f32 = 31.0;
const TEXT_PX: f32 = 36.0;
const TEXT_LINE: f32 = 56.5;
const TEXT_TOP: f32 = 8.0;
const TEXT_BOTTOM: f32 = 27.5;
const RULE_X: f32 = 21.0;
const STATUS_TOP: f32 = 16.0;
const STATUS_H: f32 = 24.0;
const PAD_BOTTOM: f32 = 21.0;
const MAX_LINES: u32 = 5;
/// Below this confidence a word gets a wavy underline.
const UNSURE: f32 = 0.6;
/// The caret blinks once no word has come for this long.
const CARET_IDLE: f64 = 0.6;

#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    ui: u32,
    mono: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Inter", SANS),
            ui: add("Inter:medium", SANS),
            mono: add("JetBrains Mono:medium", MONO),
        }
    })
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(-0.7)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    look.width - 2.0 * TEXT_X
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

fn panel_h(lines: u32) -> f32 {
    PAD_TOP
        + CHIP_H
        + WAVE_GAP
        + WAVE_H
        + TEXT_TOP
        + lines as f32 * TEXT_LINE
        + TEXT_BOTTOM
        + 1.0
        + STATUS_TOP
        + STATUS_H
        + PAD_BOTTOM
}

pub(crate) fn surface(look: &Look) -> Size {
    Size::new(look.width.ceil() + 24.0, (panel_h(MAX_LINES) + 24.0).ceil())
}

/// A small line icon on the 24 unit grid, stroked in `color`.
fn icon(key: &'static str, outline: Outline, px: f32, width: f32, color: Color) -> Element<Msg> {
    let outline = Arc::new(outline);
    Element::stroke_path(
        key,
        outline,
        Rect::new(0.0, 0.0, 24.0, 24.0),
        StrokeStyle::solid(width, color),
    )
    .size(px, px)
}

fn contours(c: Vec<Contour>) -> Outline {
    Outline { contours: c }
}

/// A keyboard: a rounded box with two rows of keys and a space bar.
fn keyboard() -> Outline {
    let mut c = vec![make::panel(3.0, 6.0, 18.0, 12.0, 2.0)];
    for (y, n) in [(9.5, 5), (12.5, 5)] {
        for i in 0..n {
            let x = 6.0 + i as f32 * 3.0;
            c.push(Contour::polyline(
                &[Point::new(x, y), Point::new(x + 0.5, y)],
                false,
            ));
        }
    }
    c.push(Contour::polyline(
        &[Point::new(8.5, 15.5), Point::new(15.5, 15.5)],
        false,
    ));
    contours(c)
}

/// A small chevron pointing down.
fn chevron() -> Outline {
    contours(vec![Contour::polyline(
        &[
            Point::new(7.0, 9.5),
            Point::new(12.0, 14.5),
            Point::new(17.0, 9.5),
        ],
        false,
    )])
}

/// The five rounded bars of the logo, in a blue to lilac ramp.
fn logo(t: f64, r: Rect, p: &mut Painter, live: bool) {
    let heights = [0.35, 0.7, 1.0, 0.7, 0.35];
    let pitch = r.w / 5.0;
    for (i, h) in heights.iter().enumerate() {
        let wobble = if live && !reduced() {
            0.8 + 0.2 * ((t * 5.0 + i as f64 * 1.3).sin() as f32)
        } else {
            1.0
        };
        let bh = r.h * h * wobble;
        let x = r.x + pitch * (i as f32 + 0.5);
        let c = mix(hex(VIOLET), hex(LILAC), i as f32 / 4.0);
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(x - 1.2, r.y + (r.h - bh) / 2.0, 2.4, bh), 1.2),
            Paint::solid(c),
        ));
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

/// The bar waveform, mirrored out from the middle.
#[derive(Clone)]
struct Bars {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
}

impl Bars {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => f64::INFINITY,
            _ => self.mode_at + 0.5,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = r.w / DESIGN_W;
        let cy = r.y + r.h / 2.0;
        let x0 = r.x + BAR_X0 * k;
        let x1 = r.x + BAR_X1 * k;
        let mid = (x0 + x1) / 2.0;
        let n = ((x1 - x0) / (BAR_PITCH * k) / 2.0).floor() as i32;
        let since = t - self.shown_at;
        let open = if reduced() {
            1.0
        } else {
            ease_out((since - 0.12) / 0.3)
        };
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let dim = match self.mode {
            Mode::Finishing | Mode::Done => 1.0 - 0.6 * ease_out((t - self.mode_at) / 0.3),
            Mode::Cancelled => 0.3,
            _ => 1.0,
        };
        let sweep = ((since / 1.2).fract()) as f32 * 2.0 - 1.0;
        for i in -n..=n {
            let reach = i.unsigned_abs() as f32 / n as f32;
            if reach > open + 1e-3 {
                continue;
            }
            let x = mid + i as f32 * BAR_PITCH * k;
            // Blue at the ends, violet in the middle.
            let c = mix(hex(VIOLET), hex(BLUE), (reach * 1.6).min(1.0)).fade(dim);
            if self.mode == Mode::Loading {
                let u = i as f32 / n as f32;
                let near = (1.0 - (u - sweep).abs() * 6.0).clamp(0.0, 1.0);
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.2 + 1.0 * near,
                    Paint::solid(c.fade(0.4 + 0.6 * near)),
                ));
                continue;
            }
            let at = t_eff - i.unsigned_abs() as f64 * BAR_SECS;
            let jitter = 0.5 + 0.5 * noise((at / BAR_SECS).round() as i64 as u32);
            let level = self.levels.at(at) * jitter;
            let taper = (1.0 - reach.powf(1.8)).max(0.0);
            let half = (level * 1.8).min(1.0) * taper * WAVE_HALF * k;
            if half < 2.0 {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.3,
                    Paint::solid(c.fade(0.8)),
                ));
                continue;
            }
            let bar = Rect::new(x - BAR_W / 2.0, cy - half, BAR_W, 2.0 * half);
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W / 2.0),
                Paint::glow(c.fade(0.22), 4.0, 0.5),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W / 2.0),
                Paint::solid(c),
            ));
        }
    }
}

/// The status mark: a teal dot with a ring that spreads, a spinner, a check
/// in a ring, or an X.
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
        let k = ease_out((t - self.mode_at) / 0.25);
        match self.mode {
            Mode::Loading | Mode::Listening => {
                if !reduced() {
                    let u = ((t / 1.5).fract()) as f32;
                    let ring = make::ring(c, s * (0.45 + 0.55 * u));
                    p.stroke(
                        &ring,
                        &StrokeStyle::solid(1.2, hex(TEAL).fade(0.6 * (1.0 - u))),
                    );
                }
                p.shape(ShapeInstance::circle(c, s * 0.5, Paint::solid(hex(TEAL))));
            }
            Mode::Finishing => {
                p.stroke(
                    &make::spinner(c, s * 0.6, t),
                    &StrokeStyle::solid(1.6, hex(TEAL)),
                );
            }
            Mode::Done => {
                p.stroke(
                    &make::progress(c, s * 0.75, k),
                    &StrokeStyle::solid(1.4, hex(TEAL)),
                );
                let check = Outline::new(&icons::check(), 0.05).transform(
                    &Affine::scale(s * 1.0 / 24.0)
                        .then(&Affine::translate(c.x - s * 0.5, c.y - s * 0.5)),
                );
                let len = check.length();
                p.outline(
                    &check.trim(0.0, len * k),
                    &StrokeStyle::solid(1.4, hex(TEAL)),
                );
            }
            Mode::Cancelled => {
                let x = Outline::new(&icons::x(), 0.05).transform(
                    &Affine::scale(s * 1.2 / 24.0)
                        .then(&Affine::translate(c.x - s * 0.6, c.y - s * 0.6)),
                );
                let len = x.length();
                p.outline(&x.trim(0.0, len * k), &StrokeStyle::solid(1.5, hex(GREY)));
            }
        }
    }
}

impl Popup {
    pub(crate) fn palette_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.palette_panel().margin_edges(12.0, 0.0, 0.0, 0.0))
    }

    pub(crate) fn palette_panel(&self) -> Element<Msg> {
        let l = &self.look;
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let bars = Bars {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
        };
        let until = bars.until();
        let wave = Element::canvas("bars", until, move |t, r, p| bars.draw(t, r, p))
            .size(l.width, WAVE_H)
            .margin_edges(WAVE_GAP, 0.0, 0.0, 0.0);
        let rule_w = l.width - 2.0 * RULE_X;
        Element::panel("panel")
            .width(l.width)
            .pad_edges(PAD_TOP, 0.0, PAD_BOTTOM, 0.0)
            .radius(RADIUS)
            .fill(hex(CARD).fade(0.97))
            .border(hex(EDGE), 1.0)
            .shadow(Color::rgba(0.0, 0.0, 0.0, 0.5), 0.0, 6.0, 10.0)
            .presence(Transition::fade().offset(0.0, 8.0).scale(0.98), exit)
            .child(self.palette_chips(mode))
            .child(wave)
            .child(self.palette_text(mode))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .size(rule_w, 1.0)
                    .fill(hex(RULE))
                    .margin_edges(0.0, 0.0, 0.0, RULE_X),
            )
            .child(self.palette_progress(mode, rule_w))
            .child(self.palette_status(mode, mode_at))
    }

    /// The logo, the three chips and the gear.
    fn palette_chips(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let style = TextStyle::new(f.ui, 12.8, 16.0);
        let chip = |key: &'static str, n: usize| {
            Element::panel(key)
                .direction_row()
                .height(CHIP_H)
                .align_items(AlignItems::CENTER)
                .pad_edges(0.0, 12.0, 0.0, 10.0)
                .gap(7.0)
                .radius(CHIP_H / 2.0)
                .fill(hex(CHIP))
                .border(hex(CHIP_EDGE), 1.0)
                .margin_edges(0.0, 0.0, 0.0, if n == 0 { 14.0 } else { 12.0 })
                // Chips slide in one after another as the popup opens.
                .presence(
                    Transition::fade().offset(-10.0 - 6.0 * n as f32, 0.0),
                    Transition::fade(),
                )
        };
        let text =
            |key: &'static str, s: String| Element::label(key, s).text_style(style).color(hex(INK));
        let facts = facts();
        let live = matches!(mode, Mode::Listening | Mode::Loading);
        let logo = Element::canvas(
            "logo",
            if live { f64::INFINITY } else { 0.0 },
            move |t, r, p| logo(t, r, p, live),
        )
        .size(18.0, 18.0)
        .margin_edges(0.0, 0.0, 0.0, 22.0);
        let engine = chip("engine", 0)
            .child(icon(
                "icon",
                Outline::new(&icons::chip(), 0.05),
                15.0,
                1.8,
                hex(BLUE),
            ))
            .child(text("name", facts.engine.clone()))
            .child(icon("more", chevron(), 12.0, 2.0, hex(SOFT)));
        let output = chip("output", 1)
            .child(icon("icon", keyboard(), 16.0, 1.6, hex(INK)))
            .child(text("name", facts.output.clone()))
            .child(icon("more", chevron(), 12.0, 2.0, hex(SOFT)));
        let latency = self
            .fam
            .latency
            .map_or("0.0".to_string(), |s| format!("{s:.1}"));
        let latency = chip("latency", 2)
            .child(icon(
                "icon",
                Outline::new(&icons::timer(), 0.05),
                15.0,
                1.8,
                hex(INK),
            ))
            .child(
                Element::row("value")
                    .align_items(AlignItems::CENTER)
                    .child(digit_roll(
                        "digits",
                        &latency,
                        style,
                        hex(INK),
                        Spring::critical_to_98(0.3),
                    ))
                    .child(text("unit", " s latency".into())),
            );
        let gear = Element::panel("gear")
            .direction_row()
            .size(32.0, 32.0)
            .align_items(AlignItems::CENTER)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .radius(16.0)
            .border(hex(CHIP_EDGE), 1.0)
            .child(icon(
                "icon",
                Outline::new(&icons::gear(), 0.05),
                17.0,
                1.7,
                hex(INK),
            ))
            .margin_edges(0.0, 18.0, 0.0, 0.0);
        Element::row("chips")
            .height(CHIP_H)
            .align_items(AlignItems::CENTER)
            .child(logo)
            .child(engine)
            .child(output)
            .child(latency)
            .child(Element::row("spacer").grow(1.0))
            .child(gear)
    }

    /// The words the engine is unsure of, as long as no final pass began.
    fn palette_unsure(&self) -> Vec<u32> {
        if self.extra.finish_at.is_some() {
            return Vec::new();
        }
        self.shown
            .iter()
            .filter(|w| {
                self.model
                    .info(w.id)
                    .and_then(|i| i.conf)
                    .is_some_and(|c| c < UNSURE)
            })
            .map(|w| w.id)
            .collect()
    }

    fn palette_text(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let style = l.text_style();
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .text_style(style)
            .color(hex(INK))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0));
        let words = self.handoff_or(words, &style, Some(l.text_width()), hex(INK), hex(VIOLET));
        let mut text = Element::column("text")
            .height(self.lines.min(MAX_LINES) as f32 * TEXT_LINE)
            .margin_edges(TEXT_TOP, TEXT_X, TEXT_BOTTOM, TEXT_X)
            .child(words)
            .children(self.line_marks(
                style,
                l.text_width(),
                endings::Ink {
                    text: hex(INK),
                    mark: hex(VIOLET),
                    faint: hex(GREY),
                },
                endings::Cancel::Unthread,
            ));
        // Lay the words out as the live text does, to place the wavy lines
        // and the caret.
        let joined = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(l.text_width()));
        let unsure = self.palette_unsure();
        let mut start = 0usize;
        for w in &self.shown {
            let range = start..start + w.text.len();
            start = range.end + 1;
            if !unsure.contains(&w.id) {
                continue;
            }
            let Some(first) = laid
                .glyphs
                .iter()
                .find(|g| range.contains(&(g.cluster as usize)))
            else {
                continue;
            };
            let width = self.measure.layout(&w.text, &style, None).size.w;
            let line = Element::wavy_line(
                format!("unsure-{}", w.id),
                StrokeStyle::solid(1.3, hex(DOUBT)),
                1.6,
                6.0,
            )
            .size(width, 6.0)
            .opacity(0.9)
            .motion(Prop::Trim, Spring::critical_to_98(0.25))
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade());
            text = text.child(at(line, first.pos.x, first.pos.y + 3.0));
            // The word itself in a soft blue over the white.
            let top = (first.pos.y / TEXT_LINE).floor() * TEXT_LINE;
            let tint = Element::label(format!("tint-{}", w.id), w.text.as_str())
                .text_style(style)
                .color(hex(DOUBT))
                .presence(Transition::fade(), Transition::fade());
            text = text.child(at(tint, first.pos.x, top));
        }
        // The caret after the last word: it jumps as a text field's does and
        // blinks once the words stop coming.
        if !matches!(mode, Mode::Done | Mode::Cancelled) {
            let (x, y) = match laid.glyphs.last() {
                Some(g) => {
                    let tail = joined.get(g.cluster as usize..).unwrap_or("");
                    let w = self.measure.layout(tail, &style, None).size.w;
                    (g.pos.x + w + 5.0, g.pos.y - TEXT_PX * 0.78)
                }
                None => (0.0, (TEXT_LINE - TEXT_PX) / 2.0),
            };
            let idle_from = self.fam.preview_at + CARET_IDLE;
            let caret = Element::canvas("caret", f64::INFINITY, move |t, r, p| {
                let on = reduced() || t < idle_from || ((t - idle_from) / 0.53) as i64 % 2 == 0;
                if on {
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(r.x, r.y, 2.4, r.h), 1.2),
                        Paint::solid(hex(VIOLET)),
                    ));
                }
            })
            .size(3.0, TEXT_PX * 1.02)
            .offset(x, y)
            .presence(Transition::fade(), Transition::fade());
            text = text.child(at(caret, 0.0, 0.0));
        }
        text
    }

    /// A thin violet line under the rule while a model loads or the final
    /// text is made.
    fn palette_progress(&self, mode: Mode, w: f32) -> Element<Msg> {
        let row = Element::row("progress-row")
            .height(2.0)
            .margin_edges(0.0, 0.0, 0.0, RULE_X);
        match self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))
        {
            Some(fraction) => row.child(
                Element::progress(
                    "progress",
                    Progress {
                        fraction: Some(fraction),
                        track: Color::TRANSPARENT,
                        bar: hex(VIOLET),
                        since: self.shown_at,
                        period: 1.6,
                    },
                )
                .size(w, 2.0)
                .presence(Transition::fade(), Transition::fade()),
            ),
            None => row,
        }
    }

    fn palette_status(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let big = TextStyle::new(f.ui, 15.5, 20.0);
        let small = TextStyle::new(f.ui, 14.0, 20.0);
        let status = self.status();
        let mark = Mark { mode, mode_at };
        let until = mark.until();
        let state = match mode {
            Mode::Done => self.delivery_word().to_string(),
            Mode::Cancelled => "Cancelled".to_string(),
            _ => status.state.clone(),
        };
        let mut row = Element::row("status")
            .height(STATUS_H)
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_TOP - 2.0, 0.0, 0.0, 24.0)
            .child(
                Element::canvas("mark", until, move |t, r, p| mark.draw(t, r, p)).size(18.0, 18.0),
            )
            .child(
                Element::label(format!("state-{state}"), state.as_str())
                    .text_style(big)
                    .color(hex(INK))
                    .margin_edges(0.0, 0.0, 0.0, 12.0)
                    .presence(Transition::fade().offset(0.0, 6.0), Transition::fade()),
            );
        let grey = |key: String, s: &str| {
            Element::label(key, s.to_string())
                .text_style(small)
                .color(hex(SOFT))
                .margin_edges(0.0, 0.0, 0.0, 9.0)
                .presence(Transition::fade().offset(0.0, 6.0), Transition::fade())
        };
        let badge = |key: String, s: &str| {
            Element::panel(key)
                .direction_row()
                .height(23.0)
                .align_items(AlignItems::CENTER)
                .pad_edges(0.0, 8.0, 0.0, 8.0)
                .radius(4.0)
                .fill(hex(BADGE))
                .border(hex(CHIP_EDGE), 1.0)
                .margin_edges(0.0, 0.0, 0.0, 9.0)
                .presence(Transition::fade().offset(0.0, 6.0), Transition::fade())
                .child(
                    Element::label("key", s.to_string())
                        .text_style(TextStyle::new(f.mono, 12.0, 16.0))
                        .color(hex(INK)),
                )
        };
        let detail = match mode {
            Mode::Done => self
                .kit_target()
                .map(|a| format!("into {a}"))
                .unwrap_or_default(),
            Mode::Cancelled => "nothing was typed".to_string(),
            _ => status
                .detail
                .trim_start_matches([' ', '·'])
                .trim()
                .to_string(),
        };
        if detail.is_empty() {
            return row;
        }
        row = row.child(grey(format!("dot-{state}"), "·"));
        // "tap Right Ctrl to stop": the key goes in a badge.
        let key = detail
            .strip_suffix(" to stop")
            .and_then(|head| head.split_once(' '))
            .filter(|_| mode == Mode::Listening);
        match key {
            Some((verb, key)) => row
                .child(grey(format!("verb-{verb}"), verb))
                .child(badge(format!("key-{key}"), key))
                .child(grey("stop".into(), "to stop")),
            None => match (mode, self.kit_target()) {
                (Mode::Done, Some(app)) => row
                    .child(grey("into".into(), "into"))
                    .child(badge(format!("app-{app}"), app)),
                _ => row.child(grey(format!("detail-{state}"), &detail)),
            },
        }
    }
}
