//! The `dotmatrix` look of the vo popup, after mock-up 21: white dots on
//! black in the manner of a dot-matrix display. The waveform is columns of
//! dots that grow out of a row of dots, the status is set in a dot-matrix
//! face and scrambles into each new state, the transcript is a clean
//! grotesk, and one red dot pulses while recording.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::kit::{ease_out, hex, mix, reduced, Levels, Mode};
use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::Point;

/// Colours sampled from the mock-up.
const RED: u32 = 0xFD4A46;
const DOT: u32 = 0xE6E8E6;
const INK: u32 = 0xF2F3F2;
const STATUS: u32 = 0xA9ADAD;
const BODY: u32 = 0x030405;
const EDGE: u32 = 0x2B323E;
const GREY: u32 = 0x5C6066;

/// The panel's width in the mock-up's units; the look scales to the popup.
const DESIGN_W: f32 = 600.0;
const RADIUS: f32 = 18.0;
/// The dot waveform: its box, the columns' pitch and the rows'.
const WAVE_TOP: f32 = 20.0;
const WAVE_H: f32 = 96.0;
const COL_X0: f32 = 28.0;
const COL_X1: f32 = 568.0;
const COL_PITCH: f32 = 9.4;
const ROW_PITCH: f32 = 8.4;
/// Dots above (and below) the baseline at full level.
const ROWS: usize = 5;
const DOT_R: f32 = 2.05;
/// Seconds of audio between neighbouring columns.
const COL_SECS: f64 = 0.03;
/// The transcript: Inter Regular.
const TEXT_X: f32 = 27.0;
const TEXT_PX: f32 = 33.5;
const TEXT_LINE: f32 = 49.6;
const TEXT_TOP: f32 = 7.0;
/// The status row: the red dot, then the dot-matrix words.
const STATUS_TOP: f32 = 43.6;
const STATUS_H: f32 = 20.0;
const STATUS_PX: f32 = 20.0;
const PAD_BOTTOM: f32 = 34.0;
const MAX_LINES: u32 = 5;

/// Seconds per letter as the status resolves from scrambled glyphs, and
/// between two scrambles of a letter.
const RESOLVE_SECS: f64 = 0.018;
const SCRAMBLE_SECS: f64 = 0.045;
/// The baseline lights from the centre, this long per column.
const LIGHT_SECS: f64 = 0.006;

/// The fonts the look adds to the registry.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    dots: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Inter", SANS),
            dots: add("Doto:extralight", MONO),
        }
    })
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(0.15)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    look.width - 2.0 * TEXT_X
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

fn panel_h(lines: u32) -> f32 {
    WAVE_TOP + WAVE_H + TEXT_TOP + lines as f32 * TEXT_LINE + STATUS_TOP + STATUS_H + PAD_BOTTOM
}

pub(crate) fn surface(look: &Look) -> Size {
    Size::new(look.width.ceil(), (panel_h(MAX_LINES) + 8.0).ceil())
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// `text` as it shows `since` seconds after it began to resolve: letters up
/// to the front are real, the rest are scrambled glyphs that change.
fn scrambled(text: &str, since: f64) -> String {
    const POOL: &[u8] = b"#%&*+=?@$<>/\\0123456789ABCDEFHKMNRSTXZ";
    if reduced() || since < 0.0 {
        return if since < 0.0 && !reduced() {
            String::new()
        } else {
            text.to_string()
        };
    }
    let front = (since / RESOLVE_SECS) as usize;
    let step = (since / SCRAMBLE_SECS) as u32;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < front || c == ' ' || c == '·' {
                c
            } else if i > front + 10 {
                ' '
            } else {
                let k = noise(step.wrapping_mul(131) ^ i as u32) * POOL.len() as f32;
                POOL[(k as usize).min(POOL.len() - 1)] as char
            }
        })
        .collect()
}

/// The dot waveform.
#[derive(Clone)]
struct Columns {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    progress: Option<f32>,
}

impl Columns {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing => f64::INFINITY,
            _ => self.mode_at + 0.6,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = r.w / DESIGN_W;
        let cy = r.y + r.h / 2.0;
        let x0 = r.x + COL_X0 * k;
        let x1 = r.x + COL_X1 * k;
        let n = ((x1 - x0) / (COL_PITCH * k)).floor() as usize + 1;
        let mid = (n - 1) as f32 / 2.0;
        let since = t - self.shown_at;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let drain = match self.mode {
            Mode::Done | Mode::Cancelled => 1.0 - ease_out((t - self.mode_at) / 0.4),
            _ => 1.0,
        };
        // A cancel switches the baseline off from both ends to the centre.
        let off = if self.mode == Mode::Cancelled {
            ease_out((t - self.mode_at) / 0.4) * mid
        } else {
            -1.0
        };
        let head = ((since / 1.2).fract() * n as f64) as f32;
        for i in 0..n {
            let from_mid = (i as f32 - mid).abs();
            // The baseline lights from the centre outward as the popup opens.
            if !reduced() && (since as f32) < from_mid * LIGHT_SECS as f32 {
                continue;
            }
            if mid - from_mid < off {
                continue;
            }
            let x = x0 + i as f32 * COL_PITCH * k;
            let dim = match (self.mode, self.progress) {
                (Mode::Finishing, Some(f)) if i as f32 / n as f32 > f => 0.25,
                (Mode::Cancelled, _) => 0.4,
                _ => 1.0,
            };
            if self.mode == Mode::Loading {
                // One lit dot runs along the baseline with a short tail.
                let back = (head - i as f32).rem_euclid(n as f32);
                let lit = (1.0 - back / 5.0).max(0.0);
                let c = mix(hex(GREY), hex(DOT), lit);
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.5 + 0.6 * lit,
                    Paint::solid(c),
                ));
                continue;
            }
            // The newest audio is at the right; older columns move left.
            let at = t_eff - (n - 1 - i) as f64 * COL_SECS;
            let jitter = 0.8 + 0.2 * noise((at / COL_SECS).round() as i64 as u32);
            let level = (self.levels.at(at) * jitter).min(1.0) * drain;
            // Quiet columns are the baseline alone, as in the mock-up.
            let rows = (((level - 0.3) / 0.55).clamp(0.0, 1.0) * ROWS as f32).round() as usize;
            let c = hex(DOT).fade(dim);
            let base_r = if rows == 0 { 1.5 } else { DOT_R };
            p.shape(ShapeInstance::circle(
                Point::new(x, cy),
                base_r * k.min(1.0),
                Paint::solid(c),
            ));
            for row in 1..=rows {
                let u = row as f32 / ROWS as f32;
                let rr = DOT_R * (1.0 - 0.3 * u) * k.min(1.0);
                let dy = row as f32 * ROW_PITCH * k;
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy - dy),
                    rr,
                    Paint::solid(c),
                ));
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy + dy),
                    rr,
                    Paint::solid(c),
                ));
            }
        }
    }
}

/// The status mark: a red dot that pulses, a ring of dots that turns, a
/// white dot, or a grey one.
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
        match self.mode {
            Mode::Loading | Mode::Listening => {
                let beat = if reduced() {
                    1.0
                } else {
                    0.5 + 0.5 * (t * std::f64::consts::TAU).cos() as f32
                };
                p.shape(ShapeInstance::circle(
                    c,
                    s * 1.1,
                    Paint::glow(hex(RED).fade(0.35 * beat), 6.0, 0.6),
                ));
                p.shape(ShapeInstance::circle(
                    c,
                    s * (0.85 + 0.15 * beat),
                    Paint::solid(hex(RED)),
                ));
            }
            Mode::Finishing => {
                let lit = ((t / 0.1) as i64).rem_euclid(8) as usize;
                for i in 0..8 {
                    let a = i as f32 / 8.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let q = Point::new(c.x + a.cos() * s * 0.8, c.y + a.sin() * s * 0.8);
                    let on = if i == lit { 1.0 } else { 0.3 };
                    p.shape(ShapeInstance::circle(
                        q,
                        1.6,
                        Paint::solid(hex(DOT).fade(on)),
                    ));
                }
            }
            Mode::Done => {
                let k = ease_out((t - self.mode_at) / 0.25);
                p.shape(ShapeInstance::circle(
                    c,
                    s * 0.9 * k.max(0.3),
                    Paint::solid(hex(DOT)),
                ));
            }
            Mode::Cancelled => {
                p.shape(ShapeInstance::circle(c, s * 0.85, Paint::solid(hex(GREY))));
            }
        }
    }
}

impl Popup {
    pub(crate) fn dotmatrix_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.dotmatrix_panel())
    }

    pub(crate) fn dotmatrix_panel(&self) -> Element<Msg> {
        let l = &self.look;
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let columns = Columns {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
            progress: self.kit_progress(),
        };
        let until = columns.until();
        let wave = Element::canvas("dots", until, move |t, r, p| columns.draw(t, r, p))
            .size(l.width, WAVE_H)
            .margin_edges(WAVE_TOP, 0.0, 0.0, 0.0);
        Element::panel("panel")
            .width(l.width)
            .pad_edges(0.0, 0.0, PAD_BOTTOM, 0.0)
            .radius(RADIUS)
            .fill(hex(BODY).fade(0.97))
            .border(hex(EDGE), 1.0)
            .presence(Transition::fade().offset(0.0, 6.0), exit)
            .child(wave)
            .child(self.dotmatrix_text(mode))
            .child(self.dotmatrix_status(mode, mode_at))
    }

    fn dotmatrix_text(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .text_style(l.text_style())
            .color(hex(INK))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.14))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0))
            .word_change(Transition::fade().opacity(0.35).offset(0.0, -4.0));
        Element::column("text")
            .height(self.lines.min(MAX_LINES) as f32 * TEXT_LINE)
            .margin_edges(TEXT_TOP, TEXT_X, 0.0, TEXT_X)
            .child(words)
    }

    fn dotmatrix_status(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(fonts(&l.fonts).dots, STATUS_PX, STATUS_H + 4.0);
        let status = self.status();
        let text = match mode {
            Mode::Done => match self.kit_target() {
                Some(app) => format!("Pasted · {app}"),
                None => "Pasted".to_string(),
            },
            Mode::Cancelled => "Cancelled · nothing was typed".to_string(),
            _ => format!("{}{}", status.state, status.detail.replace("  ·  ", " · ")),
        };
        // The label scrambles in when the state changes; the detail of the
        // same state (a number counting down) changes in place.
        let since = self.extra.now - mode_at.max(self.shown_at + 0.1);
        let shown = scrambled(&text, since);
        let mark = Mark { mode, mode_at };
        let until = mark.until();
        Element::row("status")
            .height(STATUS_H)
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_TOP, 0.0, 0.0, 30.0)
            .child(
                Element::canvas("mark", until, move |t, r, p| mark.draw(t, r, p)).size(17.0, 17.0),
            )
            .child(
                Element::label("state", shown)
                    .text_style(style)
                    .color(hex(STATUS))
                    .margin_edges(0.0, 0.0, 0.0, 12.0),
            )
    }
}
