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
//! While editing ([`CaptionsSite`]) both lines are full size and flat, the
//! newest word is part of its line, and a line keeps its left edge while
//! you type (the sticky centring of `EditView::centre`).

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, Outline, StrokeStyle};
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
                    .presence(
                        Transition::NONE,
                        if tapped {
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
        let ink = endings::Ink {
            text: hex(WHITE),
            mark: hex(ACCENT),
            faint: hex(HINT),
        };
        panel = panel.children(self.unthread_rows(&sculpt_rows, style, size, ink));
        if !label.is_empty() {
            panel = panel.child(
                at(
                    Element::label(format!("status-{}", status.state), label)
                        .text_style(ui)
                        .color(colour),
                    x + label_x,
                    y + STATUS_Y,
                )
                .presence(Transition::fade().offset(0.0, 4.0), Transition::fade()),
            );
        }
        panel
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
