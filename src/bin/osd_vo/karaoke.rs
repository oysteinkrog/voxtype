//! The `karaoke` look of the vo popup, after mock-up 14: a dark navy glass
//! card that reads like a teleprompter. The words are large, centred and
//! white; when a line fills, the older lines roll up, shrink and fade. The
//! newest word is lit in a cyan to violet gradient with a gradient underline
//! that hops to the next word. A thin mirrored waveform runs under the text,
//! and the status sits centred at the bottom with a spinning ring.
//!
//! The card, the waveform, the badge and the icons are drawn on one canvas
//! from the frame time; each line is its own live text so it can roll and
//! scale on its own spring.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

const BG_TOP: u32 = 0x0d1530;
const BG_BOTTOM: u32 = 0x0a1026;
const EDGE: u32 = 0x2b47b8;
const WHITE: u32 = 0xf2f4fa;
const TITLE: u32 = 0xd6dcec;
const STATE: u32 = 0xc3cbe0;
const HINT: u32 = 0xa7b0c8;
const CYAN: u32 = 0x19a7ff;
const BLUE: u32 = 0x3b6dff;
const VIOLET: u32 = 0x9a4dff;
const MAGENTA: u32 = 0xd23cf5;
const GREEN: u32 = 0x46d98a;

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

/// The gradient of the lit word and its underline, 0 at the left.
fn ramp(u: f32) -> Color {
    if u < 0.5 {
        mix(hex(CYAN), hex(BLUE), u * 2.0)
    } else {
        mix(hex(BLUE), hex(VIOLET), (u - 0.5) * 2.0)
    }
}

/// The card is the mock-up's 730 x 580 at 0.877, so 640 x 508; room round
/// it for the glow.
const CARD_W: f32 = 640.0;
const CARD_H: f32 = 508.0;
const MARGIN: f32 = 18.0;
const RADIUS: f32 = 18.0;

const TEXT_PX: f32 = 45.0;
const LINE: f32 = 61.0;
const WRAP: f32 = 495.0;
/// Each line's box: wider than the wrap, so a line whose words change in
/// the final pass never wraps inside it; the words sit centred in it.
const LINE_BOX: f32 = WRAP + 60.0;
/// The lines kept: three large and two small ones above them.
pub(crate) const MAX_LINES: u32 = 5;
/// Each line's centre by its age, 0 the newest, with its scale and opacity.
const SLOTS: [(f32, f32, f32); 5] = [
    (314.0, 1.0, 1.0),
    (254.0, 1.0, 1.0),
    (193.0, 1.0, 1.0),
    (138.0, 0.66, 0.36),
    (95.0, 0.62, 0.22),
];
const ROLL_SECS: f64 = 0.26;
/// How long a new line's words wait while the lines above roll up out of its slot.
const LINE_WAIT: f64 = 0.16;
/// With `bowstring` (MO-64): the underline is a string from the word before
/// to the newest word. It hangs slack when the word comes and tightens over
/// `BOW_TIGHTEN` seconds; the word's pitch bends it, up to `BOW_BEND` px.
pub(crate) const BOWSTRING: &str = "bowstring";
const BOW_SLACK: f32 = 8.0;
const BOW_BEND: f32 = 5.0;
const BOW_TIGHTEN: f64 = 0.45;

/// How far the string's middle sits below its ends at `age` seconds after
/// the word came, for a word said at `pitch` (0 to 1, `None` unvoiced):
/// slack that tightens, and a high pitch bending it up.
fn bow_sag(age: f64, pitch: Option<f32>, reduced: bool) -> f32 {
    let bend = pitch.map_or(0.0, |p| -(p - 0.5).clamp(-0.5, 0.5) * 2.0 * BOW_BEND);
    if reduced {
        return bend;
    }
    let tight = ease_out(age / BOW_TIGHTEN);
    BOW_SLACK * (1.0 - tight) + bend * tight
}

const WAVE_Y: f32 = 418.0;
const WAVE_X0: f32 = 104.0;
const WAVE_X1: f32 = 536.0;
const WAVE_HALF: f32 = 26.0;
const BARS: usize = 60;
const BAR_SECS: f64 = 0.028;
const STATUS_Y: f32 = 479.0;

const TICK_HZ: f64 = 60.0;

#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    ui: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Nunito:medium", SANS),
            ui: add("Nunito", SANS),
        }
    })
}

fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, LINE).tracking(-0.2)
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

/// State only the karaoke look keeps.
pub struct State {
    mode: Option<Mode>,
    mode_at: f64,
    /// The progress bar's eased fraction: from, to, when.
    prog: (f32, f32, f64),
}

impl Default for State {
    fn default() -> State {
        State {
            mode: None,
            mode_at: f64::NEG_INFINITY,
            prog: (0.0, 0.0, 0.0),
        }
    }
}

/// What the canvas draws.
#[derive(Clone)]
struct Scene {
    origin: Point,
    mode: Mode,
    mode_at: f64,
    shown_at: f64,
    levels: Arc<[f32]>,
    newest_at: f64,
    /// The wave's clock during a rewind cancel (MO-50).
    scrub: endings::Scrub,
    finish_at: Option<f64>,
    final_at: Option<f64>,
    /// The status ring's centre, relative to the card.
    ring: Point,
}

impl Scene {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing => now + 0.25,
            _ => (now + 0.2).max(self.mode_at + 0.6),
        }
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
        let o = self.origin;
        let card = RRect::new(Rect::new(o.x, o.y, CARD_W, CARD_H), RADIUS);
        p.shape(ShapeInstance::rrect(
            card,
            Paint::shadow(Color::BLACK.fade(0.5), Point::new(0.0, 6.0), 12.0, 0.0),
        ));
        p.shape(ShapeInstance::rrect(
            card,
            Paint::glow(hex(BLUE).fade(0.22), 8.0, 0.5),
        ));
        p.shape(
            ShapeInstance::rrect(
                card,
                Paint::linear(
                    Point::new(o.x, o.y),
                    hex(BG_TOP).fade(0.96),
                    Point::new(o.x, o.y + CARD_H),
                    hex(BG_BOTTOM).fade(0.96),
                ),
            )
            .with_stroke(
                Paint::linear(
                    Point::new(o.x, o.y),
                    hex(EDGE).fade(0.75),
                    Point::new(o.x + CARD_W, o.y + CARD_H),
                    hex(VIOLET).fade(0.55),
                ),
                1.0,
            ),
        );
        // A faint violet light in the lower right corner, as in the mock-up.
        p.shape(ShapeInstance::circle(
            Point::new(o.x + CARD_W - 60.0, o.y + CARD_H - 30.0),
            40.0,
            Paint::glow(hex(VIOLET).fade(0.10), 40.0, 0.5),
        ));
        p.layer(Affine::translate(o.x, o.y), 1.0, |p| {
            self.draw_badge(p);
            self.draw_close(p);
            self.draw_wave(t, p);
            self.draw_ring(t, p);
        });
    }

    /// The round blue badge with the microphone.
    fn draw_badge(&self, p: &mut Painter) {
        let c = Point::new(43.0, 39.0);
        let r = 17.5;
        p.shape(ShapeInstance::circle(
            c,
            r,
            Paint::glow(hex(BLUE).fade(0.35), 5.0, 0.4),
        ));
        p.shape(ShapeInstance::circle(
            c,
            r,
            Paint::linear(
                Point::new(c.x - r, c.y - r),
                hex(0x2f74f0),
                Point::new(c.x + r * 0.4, c.y + r),
                hex(0x4b2aa6),
            ),
        ));
        let o = Outline::new(&icons::mic(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(0.82))
                .then(&Affine::translate(c.x, c.y)),
        );
        // The capsule filled, the holder and stem stroked.
        if let Some(head) = o.contours.first() {
            p.fill([head], &StrokeStyle::solid(0.0, hex(WHITE)));
        }
        for c in o.contours.iter().skip(1) {
            p.stroke(c, &StrokeStyle::solid(1.6, hex(WHITE)));
        }
    }

    fn draw_close(&self, p: &mut Painter) {
        let o = Outline::new(&icons::x(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(0.78))
                .then(&Affine::translate(CARD_W - 37.0, 39.0)),
        );
        p.outline(&o, &StrokeStyle::solid(1.5, hex(TITLE)));
    }

    /// Thin bars mirrored about a line, the newest audio in the middle,
    /// blue at the ends to violet and magenta in the middle; the quiet
    /// tails are dots. After the stop the bars settle into a dotted line;
    /// the final text leaves one thin gradient line.
    fn draw_wave(&self, t: f64, p: &mut Painter) {
        let span = WAVE_X1 - WAVE_X0;
        let pitch = span / (BARS - 1) as f32;
        let t_eff = self.scrub.map(self.finish_at.map_or(t, |f| t.min(f)));
        let settle = self
            .finish_at
            .map_or(1.0, |f| 1.0 - ease_out((t - f) / 0.45));
        let flat = self.final_at.map_or(0.0, |f| ease_out((t - f) / 0.3));
        let dim = if self.mode == Mode::Cancelled { 0.35 } else { 1.0 };
        let mid = (BARS as f32 - 1.0) / 2.0;
        for i in 0..BARS {
            let d = (i as f32 - mid).abs();
            let reach = d / mid;
            let s = t_eff - d as f64 * BAR_SECS;
            let level = self.level_at(s);
            // Neighbouring bars differ; each keeps its factor as it scrolls.
            let jitter = 0.45 + 0.55 * noise((s / BAR_SECS).floor() as i64);
            let taper = (1.0 - reach.powf(2.2)).max(0.0);
            let norm = ((level * 2.4).min(1.0) * jitter * taper * settle).max(0.0);
            let half = norm * WAVE_HALF;
            let x = WAVE_X0 + i as f32 * pitch;
            // Blue at the ends, violet, then magenta towards the middle.
            let w = 1.0 - reach;
            let c = if w < 0.55 {
                mix(hex(CYAN), hex(VIOLET), w / 0.55)
            } else {
                mix(hex(VIOLET), hex(MAGENTA), (w - 0.55) / 0.45)
            }
            .fade(dim * (1.0 - flat));
            if half < 2.0 {
                p.shape(ShapeInstance::circle(
                    Point::new(x, WAVE_Y),
                    1.2,
                    Paint::solid(c.fade(0.85)),
                ));
                continue;
            }
            let r = Rect::new(x - 1.2, WAVE_Y - half, 2.4, 2.0 * half);
            p.shape(ShapeInstance::capsule(r, Paint::glow(c.fade(0.4), 3.0, 0.35)));
            p.shape(ShapeInstance::capsule(r, Paint::solid(c)));
        }
        if flat > 0.0 {
            let half = span / 2.0 * 0.78 * flat;
            let mid_x = WAVE_X0 + span / 2.0;
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(mid_x - half, WAVE_Y - 0.75, 2.0 * half, 1.5), 0.75),
                Paint::linear(
                    Point::new(mid_x - half, 0.0),
                    hex(CYAN),
                    Point::new(mid_x + half, 0.0),
                    hex(MAGENTA),
                ),
            ));
        }
    }

    /// The status ring: a spinning gradient ring while working, a green
    /// ring with a check once pasted.
    fn draw_ring(&self, t: f64, p: &mut Painter) {
        let c = self.ring;
        let r = 8.5;
        match self.mode {
            Mode::Done => {
                let k = ease_out((t - self.mode_at) / 0.3);
                p.stroke(&make::ring(c, r), &StrokeStyle::solid(1.6, hex(GREEN).fade(k)));
                let o = {
                let c = Outline::new(&icons::check(), 0.05);
                c.trim(0.0, (k) * c.length())
            }.transform(
                    &Affine::translate(-12.0, -12.0)
                        .then(&Affine::scale(0.48))
                        .then(&Affine::translate(c.x, c.y)),
                );
                p.outline(&o, &StrokeStyle::solid(1.6, hex(GREEN)));
            }
            Mode::Cancelled => {
                p.stroke(&make::ring(c, r), &StrokeStyle::solid(1.6, hex(HINT).fade(0.6)));
            }
            _ => {
                let spin = if reduced() {
                    0.0
                } else {
                    ((t - self.shown_at) / 1.4).fract() as f32
                };
                let style = StrokeStyle::solid(2.0, hex(BLUE)).conic(
                    c,
                    spin,
                    hex(CYAN),
                    hex(VIOLET),
                );
                p.stroke(&make::ring(c, r), &style);
            }
        }
    }
}

impl Popup {
    fn karaoke_mode(&self) -> Mode {
        let status = self.status();
        if self.extra.final_at.is_some() {
            Mode::Done
        } else if self.ended() {
            if self.extra.finish_at.is_some() {
                Mode::Done
            } else {
                Mode::Cancelled
            }
        } else if status.loading {
            Mode::Loading
        } else if status.finishing || self.extra.finish_at.is_some() {
            Mode::Finishing
        } else {
            Mode::Listening
        }
    }

    pub(crate) fn sync_karaoke(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Karaoke {
            return;
        }
        let mode = self.karaoke_mode();
        let k = &mut self.karaoke;
        if k.mode != Some(mode) {
            if k.mode.is_some() {
                k.mode_at = t;
            }
            k.mode = Some(mode);
        }
        let f = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
        if (f - k.prog.1).abs() > 1e-4 {
            let cur = k.prog.0 + (k.prog.1 - k.prog.0) * ease_out((t - k.prog.2) / 0.35);
            k.prog = (cur, f, t);
        }
    }

    pub(crate) fn arm_karaoke_tick(&mut self, cx: &mut Cx<Msg>) {
        let busy = cx.t() < self.karaoke.prog.2 + 0.4 && self.karaoke.mode == Some(Mode::Finishing);
        if self.look.theme == OsdTheme::Karaoke && self.extra.tick.is_none() && busy && !self.closing
        {
            self.extra.tick = Some(cx.after(1.0 / TICK_HZ, Msg::Tick));
        }
    }

    pub(crate) fn karaoke_surface(_look: &Look) -> Size {
        Size::new(
            (CARD_W + 2.0 * MARGIN).ceil(),
            (CARD_H + 2.0 * MARGIN).ceil(),
        )
    }

    pub(crate) fn karaoke_view(&self) -> Element<Msg> {
        let size = Self::karaoke_surface(&self.look);
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        root.child(self.karaoke_panel(size, 0.0, 0.0))
    }

    /// The transcript as lines, oldest first, each with its width, broken
    /// from the first word so a line keeps its words as new ones come.
    /// Notes when a new line starts. Returns how long its words wait for the roll, or
    /// `None` when nothing waits (not karaoke, the first line, the same line).
    pub(crate) fn karaoke_new_line(&mut self, t: f64) -> Option<f64> {
        if self.look.theme != OsdTheme::Karaoke {
            return None;
        }
        let lines = self.stable_lines(&self.look.text_style(), WRAP);
        let Some(first) = lines.last().and_then(|(l, _)| l.first()).map(|w| w.id) else {
            self.line_born = None;
            return None;
        };
        if self.line_born.is_some_and(|(_, id)| id == first) {
            return None;
        }
        self.line_born = Some((t, first));
        (lines.len() > 1).then_some(LINE_WAIT)
    }

    /// Whether the newest line's words still wait for the lines above to roll up.
    fn karaoke_waits(&self, first: vo_core::words::WordId, lines: usize) -> bool {
        lines > 1
            && !self.still()
            && self
                .line_born
                .is_some_and(|(at, id)| id == first && self.extra.now - at < LINE_WAIT)
    }

    pub(crate) fn stable_lines(&self, style: &TextStyle, wrap: f32) -> Vec<(Vec<Word>, f32)> {
        let all = self.model.words();
        let words = &all[all.len().saturating_sub(400)..];
        let text = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let shaped = self.measure.layout(&text, style, Some(wrap));
        let mut lines: Vec<(f32, Vec<Word>)> = Vec::new();
        let mut start = 0usize;
        let mut glyph = 0usize;
        for w in words {
            let range = start..start + w.text.len();
            start = range.end + 1;
            while glyph < shaped.glyphs.len() && (shaped.glyphs[glyph].cluster as usize) < range.start {
                glyph += 1;
            }
            let y = shaped
                .glyphs
                .get(glyph)
                .filter(|g| range.contains(&(g.cluster as usize)))
                .map_or(f32::NAN, |g| g.pos.y);
            match lines.last_mut() {
                Some((ly, line)) if y.is_nan() || (*ly - y).abs() < 1.0 => line.push(w.clone()),
                _ => lines.push((y, vec![w.clone()])),
            }
        }
        lines
            .into_iter()
            .map(|(_, line)| {
                let joined = line
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let w = self.measure.layout(&joined, style, None).size.w;
                (line, w)
            })
            .collect()
    }

    fn karaoke_panel(&self, size: Size, x: f32, y: f32) -> Element<Msg> {
        let l = &self.look;
        let k = &self.karaoke;
        let mode = k.mode.unwrap_or(Mode::Loading);
        let f = fonts(&l.fonts);
        let status = self.status();
        let (ox, oy) = (MARGIN, MARGIN);

        // The status row, centred.
        let ui = TextStyle::new(f.ui, 15.0, 20.0).tracking(0.1);
        let (state, detail, colour) = match mode {
            Mode::Done => {
                let app = self
                    .files
                    .live
                    .as_ref()
                    .and_then(|d| d.target.as_ref())
                    .map(|t| t.app.clone())
                    .filter(|a| !a.is_empty());
                let s = match app {
                    Some(app) => format!("{} into {app}", self.delivery_word()),
                    None => self.delivery_word().to_string(),
                };
                (s, String::new(), hex(GREEN))
            }
            Mode::Cancelled => ("Cancelled".to_string(), String::new(), hex(HINT)),
            _ => (status.state.clone(), status.detail.clone(), hex(STATE)),
        };
        let detail = detail.replace("  ·  ", "   ·   ");
        let sw = self.measure.layout(&state, &ui, None).size.w;
        let dw = self.measure.layout(&detail, &ui, None).size.w;
        let ring_w = 30.0;
        let row_x = (CARD_W - (ring_w + sw + dw)) / 2.0;

        let scene = Scene {
            origin: Point::new(ox, oy),
            mode,
            mode_at: k.mode_at,
            shown_at: self.shown_at,
            levels: self.wave.clone(),
            newest_at: self.newest_at,
            scrub: self.scrub(),
            finish_at: self.extra.finish_at,
            final_at: self.extra.final_at,
            ring: Point::new(row_x + 9.0, STATUS_Y),
        };
        let until = scene.until(self.extra.now);
        let canvas = at(
            Element::canvas("card", until, move |t, r, p| scene.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h);

        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.97)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let mut panel = at(Element::layer("karaoke"), x, y)
            .size(size.w, size.h)
            .presence(Transition::fade().offset(0.0, 8.0), exit)
            .child(canvas)
            .child(at(
                Element::label("title", "Voice Dictation")
                    .text_style(TextStyle::new(f.ui, 15.5, 20.0).tracking(0.1))
                    .color(hex(TITLE)),
                ox + 75.0,
                oy + 29.0,
            ));

        panel = self.karaoke_text(panel, mode, ox, oy);

        let status_row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .height(20.0)
            .child(
                Element::label(format!("state-{state}"), state.as_str())
                    .text_style(ui)
                    .color(colour)
                    .presence(Transition::fade().offset(0.0, 5.0), Transition::fade()),
            )
            .child(
                Element::label(format!("detail-{}", status.state), detail)
                    .text_style(ui)
                    .color(hex(HINT))
                    .presence(Transition::fade(), Transition::fade()),
            );
        panel.child(
            at(status_row, ox + row_x + ring_w, oy + STATUS_Y - 10.0)
                .motion(Prop::Position, Spring::critical_to_98(0.25)),
        )
    }

    /// The newest word in the gradient (a gradient box masked by the word's
    /// own glyphs, over the white word) and its gradient underline, placed
    /// in the newest line's box.
    /// MO-50: a cancel takes a line's words back into the caret, drawn in
    /// the line's own box so the line keeps its scale and fade.
    fn karaoke_rewind(&self, words: &[Word], style: &TextStyle) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let joined = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let w = self.measure.layout(&joined, style, None).size.w;
        let laid = self.lay_lines(&[(words, Point::new((LINE_BOX - w) / 2.0, 0.0))], *style);
        let ink = endings::Ink {
            text: hex(WHITE),
            mark: hex(CYAN),
            faint: hex(HINT),
        };
        self.rewind_mark("rewind", laid, ink, Size::new(LINE_BOX, LINE))
    }

    fn karaoke_hot(&self, last: &[Word], lead: f32, style: &TextStyle) -> Vec<Element<Msg>> {
        let Some(word) = last.last() else {
            return Vec::new();
        };
        let prefix = last[..last.len() - 1]
            .iter()
            .map(|w| format!("{} ", w.text))
            .collect::<String>();
        let px = lead + if prefix.is_empty() {
            0.0
        } else {
            // The advance of the prefix with its trailing space.
            self.measure.layout(&format!("{prefix}x"), style, None).size.w
                - self.measure.layout("x", style, None).size.w
        };
        let ww = self.measure.layout(&word.text, style, None).size.w;
        // The gradient runs over `span` (the whole box when `None`); past it
        // the box keeps the last colour.
        let gradient = |key: &'static str, w: f32, h: f32, span: Option<f32>| {
            Element::canvas(key, f64::NEG_INFINITY, move |_, r, p| {
                p.shape(ShapeInstance::rrect(
                    RRect::new(r, 0.0),
                    Paint::linear(
                        Point::new(r.x, 0.0),
                        ramp(0.0),
                        Point::new(r.x + span.unwrap_or(r.w), 0.0),
                        ramp(1.0),
                    ),
                ));
            })
            .size(w, h)
        };
        // A word arriving spaced out (MO-15) is wider than its plain width.
        let (box_w, span) = if self.look.on(channels::TRACKING) {
            ((ww + 4.0) * 1.3, Some(ww + 4.0))
        } else {
            (ww + 4.0, None)
        };
        let lit = at(
            Element::layer(format!("hot-{}", word.id))
                .size(box_w, LINE)
                .child(at(gradient("fill", box_w, LINE, span), 0.0, 0.0))
                .child(Element::mask("glyphs").child(at(self.karaoke_lit_glyphs(word, style), 0.0, 0.0))),
            // In the line's own box (see karaoke_text), which moves with the words.
            px - lead,
            0.0,
        )
        .motion(Prop::Position, Motion::Snap)
        // Keyed per word, so the lit copy enters with the word under it.
        .presence(Transition::fade().offset(0.0, 8.0), Transition::fade());
        if self.look.on(BOWSTRING) {
            return vec![lit, self.karaoke_bow(last, lead, px, ww, style)];
        }
        // The underline hops to the new word: its left edge moves a little
        // faster than its width, so it stretches and settles (T05).
        let under = at(
            Element::layer("under")
                .size(ww, 3.0)
                .glow(hex(BLUE).fade(0.7), 5.0, 0.7)
                .child(at(gradient("bar", ww, 3.0, None), 0.0, 0.0).radius(1.5)),
            px,
            LINE * 0.86,
        )
        .motion(Prop::Position, Spring::critical_to_98(0.18))
        .motion(Prop::Size, Spring::critical_to_98(0.24))
        .presence(Transition::fade(), Transition::fade());
        vec![lit, under]
    }

    /// The glyphs that mask the lit word's gradient. With `spoken-tracking`
    /// the word in the line arrives spaced out, so the mask is a live text
    /// with the same channels, and the two keep the same letter places.
    fn karaoke_lit_glyphs(&self, word: &Word, style: &TextStyle) -> Element<Msg> {
        if !self.look.on(channels::TRACKING) || self.look.on("reduced-motion") {
            return Element::label("word", word.text.as_str())
                .text_style(*style)
                .color(Color::WHITE);
        }
        self.typo_words(
            Element::live_text("word", std::slice::from_ref(word))
                .text_style(*style)
                .color(Color::WHITE)
                .word_enter(Transition::NONE),
        )
    }

    /// MO-64: the underline as a bowstring from the middle of the word
    /// before (or the line's start) to the end of the newest word.
    fn karaoke_bow(&self, last: &[Word], lead: f32, px: f32, ww: f32, style: &TextStyle) -> Element<Msg> {
        let word = &last[last.len() - 1];
        let x0 = match last.len() {
            0 | 1 => lead,
            n => {
                let before: String = last[..n - 2].iter().map(|w| format!("{} ", w.text)).collect();
                let start = if before.is_empty() {
                    0.0
                } else {
                    self.measure.layout(&format!("{before}x"), style, None).size.w
                        - self.measure.layout("x", style, None).size.w
                };
                let prev_w = self.measure.layout(&last[n - 2].text, style, None).size.w;
                lead + start + prev_w / 2.0
            }
        };
        let x1 = px + ww;
        let y = LINE * 0.86 + 1.5;
        let heard = self.heard.word(word.id);
        let born = heard.map_or(self.extra.now, |h| h.born);
        let pitch = heard.and_then(|h| h.pitch);
        let reduced = self.look.on("reduced-motion");
        let until = if reduced { f64::NEG_INFINITY } else { born + BOW_TIGHTEN + 0.05 };
        at(
            Element::canvas(format!("bow-{}", word.id), until, move |t, r, p| {
                let (a, b) = (Point::new(r.x + x0, r.y + y), Point::new(r.x + x1, r.y + y));
                let sag = bow_sag(t - born, pitch, reduced);
                let mid = Point::new((a.x + b.x) / 2.0, a.y + 2.0 * sag);
                let n = 24;
                let pts: Vec<Point> = (0..=n)
                    .map(|i| {
                        let u = i as f32 / n as f32;
                        let v = 1.0 - u;
                        Point::new(
                            v * v * a.x + 2.0 * u * v * mid.x + u * u * b.x,
                            v * v * a.y + 2.0 * u * v * mid.y + u * u * b.y,
                        )
                    })
                    .collect();
                let style = StrokeStyle::solid(2.6, ramp(0.0))
                    .linear(a, ramp(0.0), b, ramp(1.0))
                    .glow(4.0);
                p.stroke(&vo::scene::path::Contour::polyline(&pts, false), &style);
                // The nocks at both ends.
                for (at, u) in [(a, 0.0), (b, 1.0)] {
                    p.shape(ShapeInstance::circle(at, 2.4, Paint::solid(ramp(u))));
                }
            }),
            0.0,
            0.0,
        )
        .size(LINE_BOX, LINE + 2.0 * BOW_SLACK)
        .presence(Transition::fade(), Transition::fade())
    }

    /// A line's live text with the typographic channels (MO-15
    /// `spoken-tracking` and the others, `channels.rs`) and, on the newest
    /// line, the line motion (MO-32 `margin-mark` and the others). Each line
    /// is a live text of its own, so the mark's split on a wrap is drawn by
    /// `karaoke_mark_top`. Under reduced motion words keep plain spacing.
    fn karaoke_channels(&self, text: Element<Msg>, words: &[Word], newest: bool) -> Element<Msg> {
        let text = if self.look.on("reduced-motion") {
            text
        } else {
            self.typo_words(text)
        };
        let text = self.typo_marks_for(text, words, LINE_BOX);
        if !newest {
            return text;
        }
        let text = text.line_motion(&self.look);
        match self.look.margin_mark() {
            // In the look's own cyan rather than the desktop accent.
            Some(m) => text.margin_mark(MarginMark { color: hex(CYAN), ..m }),
            None => text,
        }
    }

    /// MO-32 on karaoke: when a new line starts, the top half of the margin
    /// mark stays beside the line before (`lines[i]`) and fades as that line
    /// rolls up; the bottom half is the newest line's own mark, in the slot the
    /// reader keeps looking at. Nothing splits under reduced motion.
    fn karaoke_mark_top(&self, lines: &[(Vec<Word>, f32)], i: usize, style: &TextStyle) -> Option<Element<Msg>> {
        let mark = self.look.margin_mark()?;
        if self.look.on("reduced-motion") || kit::reduced() {
            return None;
        }
        let newest = lines.last()?.0.first()?.id;
        let (born, id) = self.line_born?;
        if id != newest || self.extra.now - born >= mark.fade {
            return None;
        }
        let (words, w) = &lines[i];
        let joined = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let baseline = self.measure.layout(&joined, style, None).glyphs.first()?.pos.y;
        let full = mark.height * style.size;
        let x = (LINE_BOX - w) / 2.0 - mark.gap - mark.width;
        let y = baseline - 0.28 * style.size - 0.25 * full;
        let fade = mark.fade;
        Some(at(
            Element::canvas("mark-top", born + fade, move |t, r, p| {
                let k = 1.0 - ease_out((t - born) / fade);
                if k <= 0.0 {
                    return;
                }
                let bar = Rect::new(r.x + x, r.y + y - 0.25 * full, mark.width, 0.5 * full);
                p.shape(ShapeInstance::rrect(
                    RRect::new(bar, 0.5 * mark.width),
                    Paint::solid(hex(CYAN).fade(k)),
                ));
            }),
            0.0,
            0.0,
        )
        .size(LINE_BOX, LINE))
    }

    /// The rolling lines, the lit newest word and its underline, or the
    /// progress line while finishing.
    fn karaoke_text(&self, mut panel: Element<Msg>, mode: Mode, ox: f32, oy: f32) -> Element<Msg> {
        let style = self.look.text_style();
        let all = self.stable_lines(&style, WRAP);
        let lines = &all[all.len().saturating_sub(SLOTS.len())..];
        let roll = Spring::critical_to_98(ROLL_SECS);
        let n = lines.len();
        let cancelled = mode == Mode::Cancelled;
        // A new line rolls in empty, and its words come once the line before has left the slot.
        let waits = lines
            .last()
            .is_some_and(|(l, _)| self.karaoke_waits(l[0].id, all.len()));
        // The lit newest word and its underline ride on the newest line.
        let hot = match (mode, lines.last()) {
            _ if waits => Vec::new(),
            (Mode::Listening, Some((last, w))) => self.karaoke_hot(last, (LINE_BOX - w) / 2.0, &style),
            _ => Vec::new(),
        };
        let mut hot = Some(hot);
        for (i, (words, _)) in lines.iter().enumerate() {
            let age = n - 1 - i;
            let Some(&(cy, scale, opacity)) = SLOTS.get(age) else {
                continue;
            };
            let key = format!("line-{}", words[0].id);
            // Laid out but not drawn while it waits, so the line is already centred when
            // its words fade in.
            let hidden = age == 0 && waits;
            // Shows at once when the roll ends, so its first word is readable in that frame.
            let text = Element::live_text("words", words)
                .opacity(if hidden { 0.0 } else { 1.0 })
                .text_style(style)
                .color(hex(WHITE))
                .motion(Prop::Opacity, Motion::Snap)
                .motion(Prop::Offset, Spring::POP)
                .word_enter(Transition::fade().offset(0.0, 8.0));
            let text = self.karaoke_channels(text, words, age == 0);
            // The words sit in a box of their own, with the lit newest word in it, so the
            // two move as one when the line re-centres.
            let mut hot = if age == 0 {
                hot.take().unwrap_or_default()
            } else {
                Vec::new()
            };
            let mut inline = Element::column("inline").child(text.rewind_hide(self.rewinding()));
            if hidden {
                // Re-centres at once while hidden, so it does not slide when it shows.
                inline = inline.motion(Prop::Position, Motion::Snap);
            }
            if !hot.is_empty() {
                inline = inline.child(hot.remove(0));
            }
            let column = Element::column(key)
                .size(LINE_BOX, LINE)
                .align_items(AlignItems::CENTER)
                .child(inline)
                .children(self.karaoke_rewind(words, &style))
                .children(hot)
                .children((age == 1).then(|| self.karaoke_mark_top(lines, i, &style)).flatten());
            let line = at(column, ox + (CARD_W - LINE_BOX) / 2.0, oy + cy - LINE / 2.0)
                .scale(scale)
                .opacity(if cancelled { opacity * 0.4 } else { opacity })
                .motion(Prop::Position, roll)
                .motion(Prop::Size, Motion::Snap)
                .motion(Prop::Scale, roll)
                .motion(Prop::Opacity, roll)
                // A new line comes in only through its words: it waits hidden for the roll,
                // and a fade or slide of its own would still be running when they show.
                .presence(
                    Transition::NONE,
                    Transition::fade().offset(0.0, -24.0).scale(0.9),
                );
            panel = panel.child(line);
        }
        let Some((_, lw)) = lines.last() else {
            return panel;
        };
        let line_y = oy + SLOTS[0].0 - LINE / 2.0;
        let under_y = line_y + LINE * 0.86;
        // MO-72: the phrase's pitch in the right margin, beside the line.
        let margin = (CARD_W - LINE_BOX) / 2.0;
        let score = Rect::new(
            ox + CARD_W - margin + 2.0,
            line_y + LINE * 0.2,
            margin - 14.0,
            LINE * 0.5,
        );
        panel = panel.children(self.score_element(score, hex(CYAN)));
        // MO-71: the sound under the newest words, below the underline.
        if let Some((last, _)) = lines.last() {
            let x = ox + (CARD_W - LINE_BOX) / 2.0 + (LINE_BOX - lw) / 2.0;
            panel = panel.children(self.receipts_element(
                "karaoke",
                last,
                &style,
                x,
                line_y + 4.0,
                WRAP,
                hex(BLUE),
            ));
        }
        if matches!(mode, Mode::Finishing) {
            {
                {
                    let (a, b, at_t) = self.karaoke.prog;
                    let f = a + (b - a) * ease_out((self.extra.now - at_t) / 0.35);
                    let w = lw.max(300.0);
                    let x = ox + (CARD_W - w) / 2.0;
                    panel = panel
                        .child(at(
                            Element::shape("track", ShapeKind::RRect)
                                .size(w, 3.0)
                                .radius(1.5)
                                .fill(Color::WHITE.fade(0.12)),
                            x,
                            under_y + 6.0,
                        ))
                        .child(
                            at(
                                Element::layer("under")
                                    .size((w * f).max(3.0), 3.0)
                                    .glow(hex(BLUE).fade(0.6), 4.0, 0.6)
                                    .child(Element::canvas("bar", f64::NEG_INFINITY, move |_, r, p| {
                                        p.shape(ShapeInstance::rrect(
                                            RRect::new(r, 1.5),
                                            Paint::linear(
                                                Point::new(r.x, 0.0),
                                                hex(CYAN),
                                                Point::new(r.x + w, 0.0),
                                                hex(MAGENTA),
                                            ),
                                        ));
                                    })
                                    .size((w * f).max(3.0), 3.0)),
                                x,
                                under_y + 6.0,
                            )
                            .motion(Prop::Position, Spring::critical_to_98(0.3))
                            .motion(Prop::Size, Spring::critical_to_98(0.3)),
                        );
                }
            }
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bowstring_tightens_and_the_pitch_bends_it() {
        // Slack when the word comes, tight a moment later.
        assert!((bow_sag(0.0, None, false) - BOW_SLACK).abs() < 1e-4);
        assert!(bow_sag(BOW_TIGHTEN, None, false).abs() < 0.2);
        // A high word bends the string up, a low one down.
        assert!(bow_sag(1.0, Some(1.0), false) < -BOW_BEND * 0.9);
        assert!(bow_sag(1.0, Some(0.0), false) > BOW_BEND * 0.9);
        // Reduced motion: no slack, only the bend.
        assert_eq!(bow_sag(0.0, Some(0.5), true), 0.0);
    }
}
