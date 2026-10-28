//! The `swissfi` look of the vo popup, after mock-up 12: Swiss type meets
//! sci-fi. A strict grid in off-black and off-white with one orange accent,
//! huge grotesk words, a hairline waveform of ticks with an orange centre,
//! and small mono numerals for the duration and the word count.
//!
//! The motion is in the lines and the numbers: the border draws itself on,
//! the hairlines run from left to right, the ticks rise out of the baseline
//! from the centre, the clock and count roll their digits, the status square
//! breathes and turns into a spinner, then a check.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::kit::{at, ease_in_out, ease_out, hex, mix, reduced, Levels, Mode};
use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;

/// Colours sampled from the mock-up.
const ORANGE: u32 = 0xFF5A0A;
const PAPER: u32 = 0xECEFF1;
const BODY: u32 = 0x090D11;
const EDGE: u32 = 0x30353B;
const RULE: u32 = 0x252A30;
const GREY: u32 = 0x6C7076;
const MUTED: u32 = 0x9CA1A6;
const GREEN: u32 = 0x3DDC84;

/// The panel's width in the mock-up's units; the look scales to the popup.
const DESIGN_W: f32 = 600.0;
const RADIUS: f32 = 7.0;
const PAD_X: f32 = 20.0;
const PAD_TOP: f32 = 10.0;
const HEADER_H: f32 = 32.0;
/// The waveform box, and the reach of a tick above and below the baseline.
const WAVE_H: f32 = 44.0;
const WAVE_HALF: f32 = 20.0;
/// The ticks run between these x, mirrored round their middle.
const TICK_X0: f32 = 63.0;
const TICK_X1: f32 = 505.0;
const TICK_PITCH: f32 = 6.5;
const TICK_W: f32 = 1.3;
/// Seconds of audio between neighbouring ticks.
const TICK_SECS: f64 = 0.04;
/// The transcript: Inter Regular, as large as in the mock-up.
const TEXT_PX: f32 = 40.0;
const TEXT_LINE: f32 = 43.5;
const TEXT_INSET: f32 = 5.3;
const TEXT_TOP: f32 = 7.3;
const TEXT_BOTTOM: f32 = 18.7;
const STATUS_TOP: f32 = 9.0;
const STATUS_H: f32 = 14.0;
const PAD_BOTTOM: f32 = 10.0;
const MAX_LINES: u32 = 5;

/// Seconds for the border to draw on, and for a hairline to run across.
const FRAME_SECS: f64 = 0.45;
const RULE_SECS: f64 = 0.35;
const RULE_DELAY: f64 = 0.08;
/// Ticks rise out of the baseline from the centre outward.
const TICKS_DELAY: f64 = 0.15;
const TICKS_SECS: f64 = 0.3;
/// Seconds for the ticks to change colour or height at a change of mode.
const SETTLE_SECS: f64 = 0.3;
/// The word count's column at its narrowest, and the room left of it for
/// the notch its casts land in.
const COUNT_MIN_W: f32 = 32.0;
const NOTCH_ROOM: f32 = 7.0;
/// How far a new word rises, and the state label slides.
const WORD_RISE: f32 = 8.0;
const LABEL_SLIDE: f32 = 7.0;

/// The fonts the look adds to the registry.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    mono: u32,
}

/// The look's fonts, added to the shared registry once. A family that is
/// missing falls back to the popup's own fonts.
fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Inter", SANS),
            mono: add("JetBrains Mono:extralight", MONO),
        }
    })
}

/// The transcript: a huge tight grotesk.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(-1.9)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    look.width - 2.0 * (PAD_X + TEXT_INSET)
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

/// The panel's height with `lines` lines of text.
fn panel_h(lines: u32) -> f32 {
    PAD_TOP
        + HEADER_H
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
    Size::new(look.width.ceil(), (panel_h(MAX_LINES) + 8.0).ceil())
}

/// The hairline waveform: ticks mirrored out from the centre.
#[derive(Clone)]
struct Ticks {
    w: f32,
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
}

impl Ticks {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => f64::INFINITY,
            _ => self.mode_at.max(now) + SETTLE_SECS + 0.1,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = r.w / DESIGN_W;
        let cy = r.y + r.h / 2.0;
        let since = t - self.shown_at;
        // The baseline runs from left to right as the popup opens.
        let run = if reduced() {
            1.0
        } else {
            ease_in_out((since - RULE_DELAY) / RULE_SECS)
        };
        let (bx0, bx1) = (r.x + PAD_X * k, r.x + r.w - PAD_X * k);
        if run > 0.0 {
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(bx0, cy - 0.5, (bx1 - bx0) * run, 1.0), 0.0),
                Paint::solid(hex(RULE)),
            ));
        }
        let x0 = r.x + TICK_X0 * k;
        let x1 = r.x + TICK_X1 * k;
        let mid = (x0 + x1) / 2.0;
        let n = ((x1 - x0) / (TICK_PITCH * k)).floor() as i32 / 2;
        if self.mode == Mode::Loading {
            // A short orange run sweeps along the baseline.
            let u = ((since / 1.2).fract()) as f32;
            let head = bx0 + (bx1 - bx0 + 60.0 * k) * u;
            let a = (head - 60.0 * k).max(bx0);
            let b = head.min(bx1);
            if b > a {
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(a, cy - 0.75, b - a, 1.5), 0.0),
                    Paint::solid(hex(ORANGE)),
                ));
            }
            return;
        }
        // Heights freeze at the stop; the finishing pass greys and lowers
        // them, the paste brings the orange back, a cancel lays them flat.
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let change = ease_out((t - self.mode_at) / SETTLE_SECS);
        let (grey, height) = match self.mode {
            Mode::Finishing => (change, 1.0 - 0.4 * change),
            Mode::Done => (1.0 - change, 0.6 + 0.4 * change),
            Mode::Cancelled => (1.0, 0.6 * (1.0 - change)),
            _ => (0.0, 1.0),
        };
        let open = if reduced() {
            1.0
        } else {
            ease_out((since - TICKS_DELAY) / TICKS_SECS)
        };
        for i in -n..=n {
            let d = i.unsigned_abs() as f32;
            let reach = d / n as f32;
            if reach > open + 1e-3 {
                continue;
            }
            let x = mid + i as f32 * TICK_PITCH * k;
            let at = t_eff - d as f64 * TICK_SECS;
            // A fixed jitter per moment of audio, so the ticks are spiky as
            // in the mock-up and the pattern travels with the sound.
            let jitter = 0.45 + 0.55 * noise((at / TICK_SECS).round() as i64 as u32);
            let level = self.levels.at(at) * jitter;
            let taper = (1.0 - reach.powf(1.6)).max(0.0);
            let norm = (level * 1.9).min(1.0) * taper * height;
            let half = (norm * WAVE_HALF * k).max(0.6);
            // Orange in the middle, grey further out.
            let warm = 1.0 - ((reach - 0.22) / 0.2).clamp(0.0, 1.0);
            let base = mix(hex(GREY), hex(ORANGE), warm * (1.0 - grey));
            let alpha = 1.0 - 0.5 * ((reach - 0.6) / 0.4).clamp(0.0, 1.0);
            let c = base.fade(alpha);
            // Out of the middle every other tick is a dot, as in the mock-up.
            if half < 1.2 || (i % 2 != 0 && reach > 0.3) {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    0.9,
                    Paint::solid(c),
                ));
                continue;
            }
            if warm * (1.0 - grey) > 0.5 && half > 9.0 {
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x - 1.5, cy - half, 3.0, 2.0 * half), 1.5),
                    Paint::glow(hex(ORANGE).fade(0.16 * warm), 3.0, 0.5),
                ));
            }
            p.shape(ShapeInstance::rrect(
                RRect::new(
                    Rect::new(x - TICK_W / 2.0, cy - half, TICK_W, 2.0 * half),
                    0.0,
                ),
                Paint::solid(c),
            ));
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

/// The status mark: a square that breathes, a spinner, a check or an X.
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
        let s = r.w.min(r.h);
        match self.mode {
            Mode::Loading | Mode::Listening => {
                let breath = if reduced() {
                    1.0
                } else {
                    0.775 + 0.225 * (t * std::f64::consts::TAU).cos() as f32
                };
                let h = 3.6;
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(c.x - h, c.y - h, 2.0 * h, 2.0 * h), 0.5),
                    Paint::solid(hex(ORANGE).fade(breath)),
                ));
            }
            Mode::Finishing => {
                let arc = make::spinner(c, s * 0.4, t);
                p.stroke(&arc, &StrokeStyle::solid(1.6, hex(ORANGE)));
            }
            Mode::Done => {
                let k = ease_out((t - self.mode_at) / 0.2);
                p.shape(ShapeInstance::circle(
                    c,
                    s * 0.5 * (0.6 + 0.4 * k),
                    Paint::solid(hex(GREEN)),
                ));
                let check = Outline::new(&icons::check(), 0.05);
                let m = Affine::scale(s * 0.64 / 24.0)
                    .then(&Affine::translate(c.x - s * 0.32, c.y - s * 0.32));
                let check = check.transform(&m);
                let len = check.length();
                p.outline(
                    &check.trim(0.0, len * ease_out((t - self.mode_at - 0.08) / 0.2)),
                    &StrokeStyle::solid(1.5, hex(BODY)),
                );
            }
            Mode::Cancelled => {
                let x = Outline::new(&icons::x(), 0.05);
                let m = Affine::scale(s * 0.8 / 24.0)
                    .then(&Affine::translate(c.x - s * 0.4, c.y - s * 0.4));
                let x = x.transform(&m);
                let len = x.length();
                p.outline(
                    &x.trim(0.0, len * ease_out((t - self.mode_at) / 0.2)),
                    &StrokeStyle::solid(1.5, hex(MUTED)),
                );
            }
        }
    }
}

impl Popup {
    pub(crate) fn swissfi_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.swissfi_panel())
    }

    /// The whole panel, `look.width` wide and as tall as its lines.
    pub(crate) fn swissfi_panel(&self) -> Element<Msg> {
        let l = &self.look;
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let lines = self.lines.min(MAX_LINES);
        let h = panel_h(lines);
        let cancelled = mode == Mode::Cancelled;
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let inner = l.width - 2.0 * PAD_X;
        Element::panel("panel")
            .width(l.width)
            .pad_edges(PAD_TOP, PAD_X, PAD_BOTTOM, PAD_X)
            .radius(RADIUS)
            .fill(hex(BODY).fade(0.97))
            .presence(Transition::fade().offset(0.0, 6.0), exit)
            .child(self.swissfi_frame(h, cancelled))
            .child(self.swissfi_cast(h))
            .children(self.limit_line(
                Point::new(PAD_X + TEXT_INSET, PAD_TOP + HEADER_H + WAVE_H + TEXT_TOP),
                Size::new(l.width, h),
                hex(PAPER).fade(0.55),
                hex(ORANGE),
            ))
            .child(self.swissfi_header())
            .child(self.swissfi_ticks(mode, mode_at))
            .child(self.with_hearing(
                self.swissfi_text(mode),
                &l.text_style(),
                l.text_width(),
                hex(PAPER),
                hex(ORANGE),
            ))
            .child(self.swissfi_rule(inner))
            .child(self.swissfi_status(mode, mode_at))
    }

    /// The border as one line round the panel, drawn on as the popup opens
    /// and erased on cancel.
    fn swissfi_frame(&self, h: f32, cancelled: bool) -> Element<Msg> {
        let l = &self.look;
        let border = Arc::new(Outline {
            contours: vec![make::panel(0.5, 0.5, l.width - 1.0, h - 1.0, RADIUS)],
        });
        let view = Rect::new(0.0, 0.0, l.width, h);
        let exit = if cancelled {
            Transition::NONE.trim(1.0, 1.0)
        } else {
            Transition::fade()
        };
        at(
            Element::stroke_path("frame", border, view, StrokeStyle::solid(self.exhale_width(1.0), hex(EDGE))),
            0.0,
            0.0,
        )
        .size(l.width, h)
        .motion(Prop::Trim, Spring::critical_to_98(FRAME_SECS))
        .presence(Transition::NONE.trim(0.0, 0.0), exit)
    }

    fn swissfi_header(&self) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let mono = |px: f32| TextStyle::new(f.mono, px, (px * 1.3).round());
        let caption = |key: &'static str, text: &str, px: f32, tracking: f32| {
            Element::label(key, text)
                .text_style(mono(px).tracking(tracking))
                .color(hex(PAPER).fade(0.78))
        };
        let roll = Spring::critical_to_98(0.28);
        let value = mono(11.0).tracking(0.4);
        let stat = |key: &'static str, digits: Element<Msg>, name: &'static str| {
            Element::column(key)
                .gap(3.0)
                .child(digits)
                .child(caption("caption", name, 6.6, 0.5))
        };
        let clock = digit_roll(
            "clock",
            &self.clock,
            value,
            self.exhale_color(hex(PAPER), hex(PAPER).fade(0.4)),
            roll,
        );
        let words = digit_roll("count", &self.tally_count(), value, hex(PAPER), roll);
        // The pace leans forward as you speak faster and stands up when you
        // pause (MO-66).
        let (wpm, lean) = self.tally_pace();
        let cell = self.measure.layout("0", &value, None).size.w;
        let pace = tally::lean_digits("pace", &wpm.to_string(), value, hex(PAPER), cell, lean);
        let mic = Arc::new(Outline::new(&icons::mic(), 0.05));
        let mic = Element::stroke_path(
            "mic",
            mic,
            Rect::new(0.0, 0.0, 24.0, 24.0),
            StrokeStyle::solid(2.0, hex(ORANGE)),
        )
        .size(17.0, 17.0);
        let cast = self.motion("count-cast");
        let mut row = Element::row("header")
            .height(HEADER_H)
            .align_items(AlignItems::START)
            .pad_edges(4.0, 0.0, 0.0, 0.0)
            .child(mic)
            .child(
                caption("title", "VOICE DICTATION", 9.0, 1.3)
                    .margin_edges(3.0, 0.0, 0.0, 13.0)
                    .presence(Transition::fade().wipe(1.0), Transition::fade()),
            )
            .child(Element::row("spacer").grow(1.0));
        if self.motion("lean-pace") {
            row = row.child(stat("pace", pace, "WPM").min_width(24.0)).child(
                Element::shape("pace-rule", ShapeKind::RRect)
                    .size(1.0, 24.0)
                    .fill(hex(EDGE))
                    .margin_edges(0.0, 23.0, 0.0, 22.0),
            );
        }
        row.child(stat("time", clock, "DURATION"))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .size(1.0, 24.0)
                    .fill(hex(EDGE))
                    .margin_edges(0.0, 23.0, 0.0, 22.0),
            )
            .child(
                stat("words", words, "WORDS")
                    .min_width(COUNT_MIN_W)
                    .margin_edges(0.0, 0.0, 0.0, if cast { NOTCH_ROOM } else { 0.0 }),
            )
    }

    /// The word's cast and the notch by the word count (MO-65), under
    /// everything else in the panel.
    fn swissfi_cast(&self, h: f32) -> Element<Msg> {
        let l = &self.look;
        if !self.motion("count-cast") {
            return at(Element::layer("cast"), 0.0, 0.0);
        }
        let f = fonts(&l.fonts);
        let value = TextStyle::new(f.mono, 11.0, 14.0).tracking(0.4);
        let caption = TextStyle::new(f.mono, 6.6, 9.0).tracking(0.5);
        let digits = self.tally_count().len().max(1) as f32;
        let col = (self.measure.layout("0", &value, None).size.w * digits)
            .max(self.measure.layout("WORDS", &caption, None).size.w)
            .max(COUNT_MIN_W);
        let notch = Rect::new(
            l.width - PAD_X - col - NOTCH_ROOM + 2.0,
            PAD_TOP + 5.0,
            2.5,
            12.0,
        );
        let origin = Point::new(PAD_X + TEXT_INSET, PAD_TOP + HEADER_H + WAVE_H + TEXT_TOP);
        let casts = self.tally_casts(origin, notch, hex(ORANGE), false);
        at(
            Element::canvas("cast", casts.until(), move |t, r, p| casts.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(l.width, h)
    }

    fn swissfi_ticks(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let ticks = Ticks {
            w: l.width,
            levels: Levels {
                levels: self.exhale_levels(&self.kit_levels().levels),
                newest_at: self.newest_at,
            },
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
        };
        let until = ticks.until(self.extra.now);
        let w = ticks.w;
        // The canvas spans the whole panel width, under the padding.
        Element::canvas("ticks", until, move |t, r, p| ticks.draw(t, r, p))
            .size(w, WAVE_H)
            .margin_edges(0.0, 0.0, 0.0, -PAD_X)
    }

    fn swissfi_text(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .text_style(l.text_style())
            .color(hex(PAPER))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.0).offset(0.0, WORD_RISE))
            .word_change(Transition::fade().opacity(0.35).offset(0.0, -4.0));
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            hex(PAPER),
            hex(ORANGE),
        );
        Element::column("text")
            .height(self.lines.min(MAX_LINES) as f32 * TEXT_LINE)
            .margin_edges(TEXT_TOP, 0.0, TEXT_BOTTOM, TEXT_INSET)
            .child(words)
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink {
                    text: hex(PAPER),
                    mark: hex(ORANGE),
                    faint: hex(GREY),
                },
                endings::Cancel::Unthread,
            ))
    }

    /// The hairline over the status row, running from left to right.
    fn swissfi_rule(&self, w: f32) -> Element<Msg> {
        let line = Arc::new(Outline {
            contours: vec![make::underline(0.0, w, 0.5)],
        });
        Element::stroke_path(
            "rule",
            line,
            Rect::new(0.0, 0.0, w, 1.0),
            StrokeStyle::solid(1.0, hex(RULE)),
        )
        .size(w, 1.0)
        .motion(Prop::Trim, Spring::critical_to_98(RULE_SECS))
        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
    }

    fn swissfi_status(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let style = TextStyle::new(f.mono, 9.6, 13.0);
        let status = self.status();
        let (state, detail, color) = match mode {
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
                hex(MUTED),
            ),
            _ => (status.state.clone(), status.detail.clone(), hex(ORANGE)),
        };
        let mark = Mark { mode, mode_at };
        let until = mark.until();
        let icon =
            Element::canvas("mark", until, move |t, r, p| mark.draw(t, r, p)).size(12.0, 12.0);
        // The state label slides up and out as the next one slides in.
        let label_w = 0.62 * 9.6 * "Cancelled".len() as f32 + 4.0;
        let label = Element::layer("state")
            .clip()
            .size(
                label_w
                    .min(self.measure.layout(&state, &style, None).size.w + 2.0)
                    .max(10.0),
                13.0,
            )
            .margin_edges(0.0, 0.0, 0.0, 8.0)
            .child(
                at(
                    Element::label(format!("state-{state}"), state.as_str())
                        .text_style(style)
                        .color(color),
                    0.0,
                    0.0,
                )
                .presence(
                    Transition::fade().offset(0.0, LABEL_SLIDE),
                    Transition::fade().offset(0.0, -LABEL_SLIDE),
                ),
            );
        let words = super::kit::detail_words(&state, &detail);
        let detail = Element::live_text(format!("detail-{state}"), &words)
            .text_style(style)
            .color(hex(PAPER).fade(0.85))
            .height(13.0)
            .margin_edges(0.0, 0.0, 0.0, 2.0)
            .shrink(1.0)
            .min_width(0.0)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().offset(0.0, LABEL_SLIDE))
            .word_change(Transition::fade().opacity(0.2).offset(0.0, -LABEL_SLIDE))
            .presence(Transition::fade(), Transition::fade());
        let mut row = Element::row("status")
            .height(STATUS_H)
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_TOP, 0.0, 0.0, 0.0)
            .child(icon.margin_edges(0.0, 0.0, 0.0, 1.0))
            .child(label)
            .child(detail);
        match self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))
        {
            Some(fraction) => {
                let progress = Progress {
                    fraction: Some(fraction),
                    track: hex(RULE),
                    bar: hex(ORANGE),
                    since: self.shown_at,
                    period: 1.6,
                };
                row = row.child(
                    Element::progress("progress", progress)
                        .height(1.5)
                        .width(140.0)
                        .margin_edges(0.0, 0.0, 0.0, 18.0),
                );
            }
            None => {}
        }
        row = row.child(Element::row("spacer").grow(1.0));
        if mode == Mode::Done {
            if let (Some(a), Some(b)) = (self.extra.finish_at, self.extra.final_at) {
                row = row.child(
                    Element::label("took", format!("{:.1}s", (b - a).max(0.0)))
                        .text_style(style)
                        .color(hex(MUTED))
                        .margin_edges(0.0, 10.0, 0.0, 0.0)
                        .presence(Transition::fade(), Transition::fade()),
                );
            }
        }
        row.child(
            Element::shape("under", ShapeKind::RRect)
                .size(22.0, 1.0)
                .fill(hex(GREY))
                .margin_edges(7.0, 2.0, 0.0, 0.0),
        )
        .child(
            Element::label("slashes", "//")
                .text_style(style)
                .color(hex(MUTED)),
        )
    }
}
