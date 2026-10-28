//! Endings and states that play on the reading line, for the looks that lay
//! their words out with one `TextStyle` in a text column:
//!
//! - MO-52, the empty measure: no speech was heard. The breath line goes
//!   flat, two measuring marks close on nothing and the caret types
//!   "No speech", 25 ms a letter.
//! - MO-54, the status word is the loading bar: while a model loads, the
//!   reading line says "Loading" and each letter fills from outline to solid
//!   as the real progress passes it. With no progress, three segments under
//!   the word trade places. When the model is ready the word turns into
//!   "Listening", the letters both words share staying.
//! - MO-51, unthread cancel: a thread under the words pulls back from the
//!   newest word to the first and lets go of a stitch at each word boundary.
//!   The words stay as they are; the panel's exit fades them.
//! - MO-50, rewind cancel: the words go back into the caret, newest first,
//!   at 4x the spoken rate and never longer than 500 ms, while the waveform
//!   scrubs back over the same stretch of audio.
//!
//! Each is a canvas laid over the text column. With reduced motion each
//! shows its end state as a still mark.

use super::*;

use kit::{ease_in_out, ease_out};
use vo::scene::canvas::Painter;
use vo::scene::display::{GlyphInstance, Paint, ShapeInstance, NO_WORD};
use vo::scene::geom::{Affine, Point};
use vo::scene::path::{glyph_path, make, Outline, StrokeStyle};
use vo::scene::text::{PlacedGlyph, ShapedText};
use vo_core::timeline::{spoken_offsets, SpokenFit};

/// MO-52: the breath line goes flat over this long.
const FLAT_SECS: f64 = 0.3;
/// MO-52: the measuring marks close between these times after the end.
const MARKS: (f64, f64) = (0.06, 0.28);
/// MO-52: the caret starts typing, and how long each letter takes.
const TYPE_AT: f64 = 0.28;
const PER_LETTER: f64 = 0.025;

/// MO-54: a step of the progress eases in over this long.
const FILL_SECS: f64 = 0.3;
/// MO-54: without progress, a pair of segments trades places this often.
const TRADE_SECS: f64 = 0.45;
/// MO-54: "Listening" stays at full ink this long after ready, then dims to
/// a state word.
const READY_HOLD: f64 = 1.2;
const READY_DIM: f32 = 0.55;

/// MO-51: the thread fades in, then pulls back over `PULL_SECS`.
const THREAD_IN: f64 = 0.06;
const PULL_SECS: f64 = 0.42;
/// MO-51: how far under the baseline the thread runs, in ems: below the
/// descenders.
const THREAD_DROP: f32 = 0.32;

/// MO-50: the rewind plays at this multiple of the spoken rate, and never
/// takes longer than `REWIND_MAX`.
const REWIND_RATE: f32 = 4.0;
const REWIND_MAX: f32 = 0.5;
const REWIND_MIN: f32 = 0.15;

/// The colours a look draws these marks in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ink {
    /// The transcript's colour.
    pub text: Color,
    /// The look's accent, for the thread, the marks and the caret.
    pub mark: Color,
    /// A quiet colour for outlines and the flat line.
    pub faint: Color,
}

impl Ink {
    /// The swiss family's colours: foreground, accent and the inactive grey.
    pub(crate) fn of(look: &Look) -> Ink {
        Ink {
            text: look.foreground,
            mark: look.accent,
            faint: look.inactive,
        }
    }
}

/// How a look takes back a cancelled dictation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cancel {
    /// MO-51.
    Unthread,
    /// MO-50, for a look whose words sit in one text column. The timeline
    /// looks that lay out their own lines use `rewind_mark`.
    #[allow(dead_code)]
    Rewind,
}

/// What the loading word needs between messages.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    /// When the model began to load, while it loads or after.
    pub loading_since: Option<f64>,
    /// When the model was ready.
    pub ready_at: Option<f64>,
    /// The fill shown, easing from one progress step to the next.
    pub fill: Fill,
}

/// The loading fill: it eases from `from` to `to`, starting at `at`. `real`
/// is false until the daemon gives a fraction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fill {
    pub from: f32,
    pub to: f32,
    pub at: f64,
    pub real: bool,
}

impl Fill {
    pub(crate) fn value(&self, t: f64) -> f32 {
        let k = ease_out((t - self.at) / FILL_SECS);
        self.from + (self.to - self.from) * k
    }

    /// The fill moves on to `to` at `t`, from where it is shown now.
    fn step(&mut self, to: f32, t: f64) {
        if self.real && to == self.to {
            return;
        }
        let from = if self.real { self.value(t) } else { 0.0 };
        *self = Fill {
            from,
            to,
            at: t,
            real: true,
        };
    }
}

impl Popup {
    /// Notes when a model starts and stops loading, and its progress.
    pub(crate) fn sync_endings(&mut self, t: f64) {
        let loading = self.status().loading && !self.ended();
        let s = &mut self.endings;
        if loading {
            if s.loading_since.is_none() || s.ready_at.is_some() {
                *s = State {
                    loading_since: Some(t),
                    ..State::default()
                };
            }
            if let Some(f) = self.files.fraction {
                s.fill.step(f.clamp(0.0, 1.0) as f32, t);
            }
        } else if s.loading_since.is_some() && s.ready_at.is_none() {
            s.ready_at = Some(t);
        }
    }

    /// The reading-line marks of the states and endings above, to lay over a
    /// text column `w` wide that shows `self.shown` in `style`. Each plays
    /// only when its feature is on: `empty-measure`, `loading-word`,
    /// `unthread` or `rewind` (the look picks `cancel`, the one it suits).
    pub(crate) fn line_marks(
        &self,
        style: TextStyle,
        w: f32,
        ink: Ink,
        cancel: Cancel,
    ) -> Vec<Element<Msg>> {
        let mut out = Vec::new();
        let still = self.still();
        let h = self.lines.max(1) as f32 * style.line_height;
        let place = |key: &'static str, until: f64, draw: Arc<DrawFn>| {
            kit::at(
                Element::canvas(key, until, move |t, r, p| draw(t, r, p)),
                0.0,
                0.0,
            )
            .size(w, h)
        };
        match (self.ending(), self.ended_at) {
            (Some(Ending::Empty), Some(at))
                if self.shown.is_empty() && self.look.on("empty-measure") =>
            {
                let m = Measure::new(&self.measure, style, w, at, self.last_level(), ink, still);
                let until = m.until();
                out.push(place(
                    "empty-measure",
                    until,
                    Arc::new(move |t, r, p| m.draw(t, r, p)),
                ));
            }
            (Some(Ending::Cancelled), Some(at)) if !self.shown.is_empty() => match cancel {
                Cancel::Unthread if self.look.on("unthread") => {
                    let th = Thread::new(&self.lay_words(style, w), at, ink, still);
                    let until = th.until();
                    out.push(place(
                        "unthread",
                        until,
                        Arc::new(move |t, r, p| th.draw(t, r, p)),
                    ));
                }
                Cancel::Rewind if self.look.on("rewind") => {
                    let rw = self.rewind(self.lay_words(style, w), ink);
                    let until = rw.until();
                    out.push(place(
                        "rewind",
                        until,
                        Arc::new(move |t, r, p| rw.draw(t, r, p)),
                    ));
                }
                _ => {}
            },
            _ => {}
        }
        if self.shown.is_empty() && self.ending().is_none() && self.look.on("loading-word") {
            if let Some(word) = self.loading_word(style, ink) {
                let until = word.until();
                let fill = word.fill_layer(&self.measure, style, ink, still, h);
                out.push(place(
                    "loading-word",
                    until,
                    Arc::new(move |t, r, p| word.draw(t, r, p)),
                ));
                out.extend(fill);
            }
        }
        out
    }

    /// The newest smoothed level, 0 to 1.
    fn last_level(&self) -> f32 {
        self.wave.last().copied().unwrap_or(0.0).clamp(0.0, 1.0)
    }

    /// The words on screen laid out as the live text lays them out.
    pub(crate) fn lay_words(&self, style: TextStyle, w: f32) -> Laid {
        let joined = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let text = self.measure.layout(&joined, &style, Some(w));
        Laid::new(text, &self.shown, style)
    }

    /// True while a cancel rewinds the words, so the look hides its own
    /// text: the rewind draws them.
    pub(crate) fn rewinding(&self) -> bool {
        self.ending() == Some(Ending::Cancelled) && !self.shown.is_empty() && self.look.on("rewind")
    }

    /// The spoken start and end of each word on screen, when the engine
    /// gives them.
    fn spoken_times(&self) -> Vec<Option<(f32, f32)>> {
        self.shown
            .iter()
            .map(|w| {
                let i = self.model.info(w.id)?;
                Some((i.start?, i.end?))
            })
            .collect()
    }

    /// The rewind of `laid`, the words on screen as the look lays them out,
    /// timed from their spoken times when the engine gives them, else from
    /// their order.
    fn rewind(&self, laid: Laid, ink: Ink) -> Rewind {
        let (all, total) = rewind_schedule(&self.spoken_times());
        // Each laid word takes its window by id, so a look may lay out
        // only some of the words on screen, one line at a time.
        let index = |w: &WordBox| self.shown.iter().position(|s| s.id == w.id);
        let windows = laid
            .words
            .iter()
            .map(|w| {
                index(w)
                    .and_then(|i| all.get(i).copied())
                    .unwrap_or((0.0, total))
            })
            .collect();
        let holds = laid
            .words
            .iter()
            .map(|w| match index(w) {
                Some(0) => f32::INFINITY,
                Some(i) => all.get(i - 1).map_or(f32::INFINITY, |o| o.0),
                None => f32::INFINITY,
            })
            .collect();
        Rewind {
            at: self.ended_at.unwrap_or(self.extra.now),
            laid: Arc::new(laid),
            windows,
            holds,
            total,
            ink,
            still: self.still(),
        }
    }

    /// MO-50 for a look that lays out its own lines: a canvas the size of
    /// `size`, placed at its parent's origin, that draws `laid` (positions
    /// from that origin) going back into the caret. `None` unless a cancel
    /// rewinds; the look hides its own words while `rewinding()`.
    pub(crate) fn rewind_mark(
        &self,
        key: &'static str,
        laid: Laid,
        ink: Ink,
        size: Size,
    ) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let rw = self.rewind(laid, ink);
        Some(
            kit::at(
                Element::canvas(key, rw.until(), move |t, r, p| rw.draw(t, r, p)),
                0.0,
                0.0,
            )
            .size(size.w, size.h),
        )
    }

    /// The words on screen laid out one line at a time, each line's text box
    /// at its point: for looks that place their lines themselves.
    pub(crate) fn lay_lines(&self, lines: &[(&[Word], Point)], style: TextStyle) -> Laid {
        let mut all = Laid {
            text: ShapedText::default(),
            words: Vec::new(),
            style,
        };
        for &(words, at) in lines {
            let joined = words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let text = self.measure.layout(&joined, &style, None);
            let line = Laid::new(text, words, style);
            let base = all.text.glyphs.len();
            all.text
                .glyphs
                .extend(line.text.glyphs.iter().map(|g| PlacedGlyph {
                    pos: Point::new(g.pos.x + at.x, g.pos.y + at.y),
                    ..*g
                }));
            all.words.extend(line.words.iter().map(|w| WordBox {
                id: w.id,
                x0: w.x0 + at.x,
                x1: w.x1 + at.x,
                baseline: w.baseline + at.y,
                first: w.first + base,
                end: w.end + base,
            }));
        }
        all
    }

    /// The clock a look's waveform reads during a rewind (MO-50).
    pub(crate) fn scrub(&self) -> Scrub {
        let (Some(at), true) = (self.ended_at, self.rewinding()) else {
            return Scrub::default();
        };
        let times = self.spoken_times();
        let (_, total) = rewind_schedule(&times);
        let first = times
            .iter()
            .flatten()
            .map(|w| w.0)
            .fold(f32::INFINITY, f32::min);
        let last = times
            .iter()
            .flatten()
            .map(|w| w.1)
            .fold(f32::NEG_INFINITY, f32::max);
        let span = if times.iter().all(Option::is_some) && last > first {
            (last - first) as f64
        } else {
            // Without word times, go back over what the waveform holds.
            self.look.window
        };
        Scrub {
            at,
            total: total as f64,
            span,
            on: true,
            still: self.still(),
        }
    }

    /// The loading word, while a model loads and a little after.
    fn loading_word(&self, style: TextStyle, ink: Ink) -> Option<LoadingWord> {
        let s = self.endings;
        let since = s.loading_since?;
        let fonts = self.measure.fonts();
        let from = Glyphs::new(&self.measure.layout("Loading", &style, None), style, fonts);
        let (phase, to) = match s.ready_at {
            Some(at) => (
                LoadPhase::Ready { at },
                Some(Glyphs::new(
                    &self.measure.layout("Listening", &style, None),
                    style,
                    fonts,
                )),
            ),
            None => (LoadPhase::Loading, None),
        };
        Some(LoadingWord {
            phase,
            from,
            to,
            fill: s.fill,
            since,
            ink,
            still: self.still(),
        })
    }
}

/// Hides a look's own words while the rewind draws them (MO-50).
pub(crate) trait RewindHide {
    fn rewind_hide(self, on: bool) -> Self;
}

impl RewindHide for Element<Msg> {
    fn rewind_hide(self, on: bool) -> Self {
        if on {
            self.opacity(0.0).motion(Prop::Opacity, Motion::Snap)
        } else {
            self
        }
    }
}

/// The time a look's waveform shows at frame time `t`: `t`, except during
/// a rewind cancel, when it goes back over the spoken audio in step with the
/// words.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Scrub {
    at: f64,
    total: f64,
    span: f64,
    on: bool,
    still: bool,
}

impl Scrub {
    pub(crate) fn map(&self, t: f64) -> f64 {
        if !self.on || t <= self.at {
            return t;
        }
        let k = if self.still {
            1.0
        } else {
            ((t - self.at) / self.total).clamp(0.0, 1.0)
        };
        self.at - self.span * ease_in_out(k) as f64
    }
}

type DrawFn = dyn Fn(f64, Rect, &mut Painter) + Send + Sync;

/// One word's box in the laid-out text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WordBox {
    /// The word's id in the word model.
    pub id: u32,
    pub x0: f32,
    pub x1: f32,
    pub baseline: f32,
    /// Its glyphs in `Laid::text.glyphs`.
    pub first: usize,
    pub end: usize,
}

/// The words on screen, laid out, with each word's box.
#[derive(Clone, Debug)]
pub(crate) struct Laid {
    pub text: ShapedText,
    pub words: Vec<WordBox>,
    pub style: TextStyle,
}

impl Laid {
    fn new(text: ShapedText, words: &[Word], style: TextStyle) -> Laid {
        let mut boxes = Vec::new();
        let mut start = 0usize;
        for w in words {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let idx: Vec<usize> = text
                .glyphs
                .iter()
                .enumerate()
                .filter(|(_, g)| range.contains(&(g.cluster as usize)))
                .map(|(i, _)| i)
                .collect();
            let (Some(&a), Some(&b)) = (idx.first(), idx.last()) else {
                continue;
            };
            let (ga, gb) = (&text.glyphs[a], &text.glyphs[b]);
            boxes.push(WordBox {
                id: w.id,
                x0: ga.pos.x,
                x1: gb.pos.x + gb.advance,
                baseline: ga.pos.y,
                first: a,
                end: b + 1,
            });
        }
        Laid {
            text,
            words: boxes,
            style,
        }
    }
}

impl Laid {
    /// The same words moved by `(dx, dy)`.
    pub(crate) fn shifted(mut self, dx: f32, dy: f32) -> Laid {
        for g in &mut self.text.glyphs {
            g.pos = Point::new(g.pos.x + dx, g.pos.y + dy);
        }
        for w in &mut self.words {
            w.x0 += dx;
            w.x1 += dx;
            w.baseline += dy;
        }
        self
    }
}

fn glyph(g: &PlacedGlyph, o: Point, style: &TextStyle, color: Color) -> GlyphInstance {
    GlyphInstance {
        pos: Point::new(o.x + g.pos.x, o.y + g.pos.y),
        size: style.size,
        font: g.font,
        glyph: g.glyph,
        word: NO_WORD,
        color,
    }
}

// MO-52 ---------------------------------------------------------------------

/// The empty measure, from the end of a dictation that heard no speech.
struct Measure {
    at: f64,
    w: f32,
    baseline: f32,
    /// The level when the recording stopped, 0 to 1: the line's last breath.
    level: f32,
    words: ShapedText,
    style: TextStyle,
    ink: Ink,
    still: bool,
}

impl Measure {
    #[allow(clippy::too_many_arguments)]
    fn new(
        measure: &vo::ShapedLayout,
        style: TextStyle,
        w: f32,
        at: f64,
        level: f32,
        ink: Ink,
        still: bool,
    ) -> Measure {
        let words = measure.layout("No speech", &style, None);
        let baseline = words
            .glyphs
            .first()
            .map_or(style.line_height * 0.75, |g| g.pos.y);
        Measure {
            at,
            w,
            baseline,
            level,
            words,
            style,
            ink,
            still,
        }
    }

    fn until(&self) -> f64 {
        self.at + TYPE_AT + PER_LETTER * self.words.glyphs.len() as f64 + 0.3
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let e = if self.still { 10.0 } else { t - self.at };
        if t < self.at {
            return;
        }
        let (x0, y) = (r.x, r.y + self.baseline + 3.0);
        let mid = x0 + self.w / 2.0;
        // The breath line: its last breath dies away and it lies flat.
        let k = (e / FLAT_SECS).clamp(0.0, 1.0) as f32;
        let amp = (1.0 + 3.0 * self.level) * (1.0 - k) * (1.0 - k);
        let line = if amp > 0.05 {
            make::wave(x0, x0 + self.w, y, amp, 14.0, (e * 9.0) as f32)
        } else {
            make::underline(x0, x0 + self.w, y)
        };
        p.stroke(
            &line,
            &StrokeStyle::solid(1.0, self.ink.faint.fade(0.9 - 0.4 * k)),
        );
        // Two measuring marks close on the middle, on nothing.
        let close = ease_in_out((e - MARKS.0) / (MARKS.1 - MARKS.0));
        let fade = 1.0 - ((e - MARKS.1) / 0.1).clamp(0.0, 1.0) as f32;
        if fade > 0.0 {
            let tall = self.style.size * 0.8;
            let half = self.w / 2.0 * (1.0 - close);
            for x in [mid - half, mid + half] {
                p.shape(ShapeInstance::line(
                    Point::new(x, y + 2.0),
                    Point::new(x, y - tall),
                    1.25,
                    Paint::solid(self.ink.mark.fade(fade)),
                ));
            }
        }
        // The caret types "No speech", centred where the marks met.
        if e < TYPE_AT {
            return;
        }
        let n = self.words.glyphs.len();
        let typed = (((e - TYPE_AT) / PER_LETTER) as usize + 1).min(n);
        let o = Point::new(mid - self.words.size.w / 2.0, r.y);
        for g in &self.words.glyphs[..typed] {
            p.glyph(glyph(g, o, &self.style, self.ink.text.fade(0.8)));
        }
        let done = TYPE_AT + PER_LETTER * n as f64;
        let caret = 1.0 - ((e - done - 0.15) / 0.1).clamp(0.0, 1.0) as f32;
        if caret > 0.0 {
            let x = self
                .words
                .glyphs
                .get(typed)
                .map_or(self.words.size.w, |g| g.pos.x);
            p.shape(ShapeInstance::line(
                Point::new(o.x + x + 1.0, r.y + self.baseline + 2.0),
                Point::new(o.x + x + 1.0, r.y + self.baseline - self.style.size * 0.8),
                1.5,
                Paint::solid(self.ink.mark.fade(caret)),
            ));
        }
    }
}

// MO-54 ---------------------------------------------------------------------

/// A word's glyphs with their outlines, for the loading word.
#[derive(Clone)]
struct Glyphs {
    text: ShapedText,
    outlines: Vec<Outline>,
    style: TextStyle,
}

impl Glyphs {
    fn new(text: &ShapedText, style: TextStyle, fonts: &FontRegistry) -> Glyphs {
        let outlines = text
            .glyphs
            .iter()
            .map(|g| {
                let Some(face) = fonts.get(g.font) else {
                    return Outline::default();
                };
                Outline::new(&glyph_path(&face, g.glyph, style.size), 0.05)
            })
            .collect();
        Glyphs {
            text: text.clone(),
            outlines,
            style,
        }
    }

    /// Glyph `i` at `o` moved `dx`: its outline with ink `line`, then the
    /// solid glyph with ink `solid`.
    #[allow(clippy::too_many_arguments)]
    fn letter(
        &self,
        p: &mut Painter,
        i: usize,
        o: Point,
        dx: f32,
        line: f32,
        solid: f32,
        ink: &Ink,
    ) {
        let g = &self.text.glyphs[i];
        if line > 0.0 {
            let m = Affine::translate(o.x + g.pos.x + dx, o.y + g.pos.y);
            p.outline(
                &self.outlines[i].transform(&m),
                &StrokeStyle::solid(0.9, ink.text.fade(0.75 * line)),
            );
        }
        if solid > 0.0 {
            p.glyph(glyph(
                g,
                Point::new(o.x + dx, o.y),
                &self.style,
                ink.text.fade(solid),
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LoadPhase {
    Loading,
    Ready { at: f64 },
}

struct LoadingWord {
    phase: LoadPhase,
    from: Glyphs,
    to: Option<Glyphs>,
    fill: Fill,
    since: f64,
    ink: Ink,
    still: bool,
}

impl LoadingWord {
    fn until(&self) -> f64 {
        match self.phase {
            LoadPhase::Ready { at } => at + READY_HOLD + 0.5,
            LoadPhase::Loading if self.fill.real => self.since,
            LoadPhase::Loading if self.still => self.since,
            LoadPhase::Loading => f64::INFINITY,
        }
    }

    /// While the model loads: the solid word in a layer clipped to the
    /// progress, its width easing to each new step (none without progress).
    /// The canvas under it draws the outlines.
    fn fill_layer(
        &self,
        measure: &vo::ShapedLayout,
        style: TextStyle,
        ink: Ink,
        still: bool,
        h: f32,
    ) -> Option<Element<Msg>> {
        if self.phase != LoadPhase::Loading {
            return None;
        }
        let w = measure.layout("Loading", &style, None).size.w;
        let motion = if still {
            Motion::Snap
        } else {
            Spring::critical_to_98(FILL_SECS).into()
        };
        Some(kit::at(
            Element::layer("loading-fill")
                .clip()
                .size(
                    if self.fill.real {
                        w * self.fill.to
                    } else {
                        0.0
                    },
                    h,
                )
                .motion(Prop::Size, motion)
                .child(
                    Element::label("word", "Loading")
                        .text_style(style)
                        .color(ink.text),
                ),
            0.0,
            0.0,
        ))
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let o = Point::new(r.x, r.y);
        match self.phase {
            LoadPhase::Loading => self.draw_loading(t, o, p),
            LoadPhase::Ready { at } => self.draw_ready(t - at, o, p),
        }
    }

    fn draw_loading(&self, t: f64, o: Point, p: &mut Painter) {
        let word = &self.from;
        for i in 0..word.text.glyphs.len() {
            word.letter(p, i, o, 0.0, 1.0, 0.0, &self.ink);
        }
        if self.fill.real {
            return;
        }
        // No progress to show: three segments under the word trade places,
        // so the line says "working" without a number it does not have.
        let w = word.text.size.w;
        let baseline = word.text.glyphs.first().map_or(0.0, |g| g.pos.y);
        let y = o.y + baseline + 4.0;
        let slot = w / 3.0;
        let lens = [0.35, 0.65, 0.95];
        let e = if self.still { 0.0 } else { t - self.since };
        for (seg, x) in trade_places(e, TRADE_SECS).into_iter().enumerate() {
            let len = lens[seg] * (slot - 4.0);
            let cx = o.x + slot * (x + 0.5);
            p.shape(ShapeInstance::line(
                Point::new(cx - len / 2.0, y),
                Point::new(cx + len / 2.0, y),
                1.25,
                Paint::solid(self.ink.mark),
            ));
        }
    }

    fn draw_ready(&self, e: f64, o: Point, p: &mut Painter) {
        let Some(to) = &self.to else { return };
        let e = if self.still { 10.0 } else { e };
        let dim = 1.0 - (1.0 - READY_DIM) * ((e - READY_HOLD) / 0.4).clamp(0.0, 1.0) as f32;
        let one_to_one = self.from.text.glyphs.len() == "Loading".len()
            && to.text.glyphs.len() == "Listening".len();
        if !one_to_one {
            // A shaper that joins letters: cross-fade the two words.
            let k = ease_out(e / 0.2);
            for i in 0..self.from.text.glyphs.len() {
                self.from.letter(p, i, o, 0.0, 0.0, 1.0 - k, &self.ink);
            }
            for i in 0..to.text.glyphs.len() {
                to.letter(p, i, o, 0.0, 0.0, k * dim, &self.ink);
            }
            return;
        }
        // Letters only "Loading" has leave; shared letters slide to their
        // place in "Listening"; the new letters come in from the left, each
        // an outline first that then fills.
        let pairs = lcs_pairs("Loading", "Listening");
        let leave = 1.0 - ease_out(e / 0.12);
        for i in 0..self.from.text.glyphs.len() {
            if leave > 0.0 && !pairs.iter().any(|&(a, _)| a == i) {
                self.from.letter(p, i, o, 0.0, 0.0, leave, &self.ink);
            }
        }
        let slide = ease_in_out((e - 0.06) / 0.24);
        for &(a, b) in &pairs {
            let x0 = self.from.text.glyphs[a].pos.x;
            let x1 = to.text.glyphs[b].pos.x;
            let dx = (x0 - x1) * (1.0 - slide);
            to.letter(p, b, o, dx, 0.0, dim, &self.ink);
        }
        let mut rank = 0;
        for j in 0..to.text.glyphs.len() {
            if pairs.iter().any(|&(_, b)| b == j) {
                continue;
            }
            let start = 0.06 + 0.03 * rank as f64;
            rank += 1;
            let line = ease_out((e - start) / 0.06);
            let solid = ease_out((e - start - 0.04) / 0.12);
            if line <= 0.0 {
                continue;
            }
            to.letter(p, j, o, 0.0, line * (1.0 - solid), solid * dim, &self.ink);
        }
    }
}

/// Where each of three segments sits, as a slot from 0 to 2, `e` seconds
/// into the trading: every `period` one neighbouring pair trades, the left
/// pair then the right, each move eased over most of the period.
fn trade_places(e: f64, period: f64) -> [f32; 3] {
    let e = e.max(0.0);
    let n = (e / period) as usize;
    let mut slots = [0usize, 1, 2];
    let swap = |slots: &mut [usize; 3], step: usize| {
        let (a, b) = if step.is_multiple_of(2) {
            (0, 1)
        } else {
            (1, 2)
        };
        for s in slots.iter_mut() {
            if *s == a {
                *s = b;
            } else if *s == b {
                *s = a;
            }
        }
    };
    // The pattern repeats every six trades.
    for step in 0..n % 6 {
        swap(&mut slots, step);
    }
    let mut next = slots;
    swap(&mut next, n % 6);
    let k = ease_in_out((e - n as f64 * period) / (period * 0.7));
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = slots[i] as f32 + (next[i] as f32 - slots[i] as f32) * k;
    }
    out
}

/// The letters two words share, as index pairs in order: a longest common
/// subsequence, so a shared letter never crosses another.
fn lcs_pairs(a: &str, b: &str) -> Vec<(usize, usize)> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut len = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i][j] = if a[i] == b[j] {
                len[i + 1][j + 1] + 1
            } else {
                len[i + 1][j].max(len[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if len[i + 1][j] >= len[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

// MO-51 ---------------------------------------------------------------------

/// The unthread: one thread per line under the words, read as one length
/// in reading order.
struct Thread {
    at: f64,
    /// Each line's run: x from, x to, the y of the thread, and where it
    /// starts along the whole thread.
    runs: Vec<(f32, f32, f32, f32)>,
    /// Each stitch: where it sits along the thread, its point, and when the
    /// pulling end passes it.
    stitches: Vec<(f32, Point, f64)>,
    total: f32,
    ink: Ink,
    still: bool,
}

impl Thread {
    fn new(laid: &Laid, at: f64, ink: Ink, still: bool) -> Thread {
        let mut runs: Vec<(f32, f32, f32, f32)> = Vec::new();
        let mut stitches = Vec::new();
        let mut total = 0.0;
        let mut prev: Option<&WordBox> = None;
        for w in &laid.words {
            let y = w.baseline + THREAD_DROP * laid.style.size;
            let same_line = prev.is_some_and(|p| (p.baseline - w.baseline).abs() < 0.5);
            if same_line {
                let run = runs.last_mut().expect("a run for the line");
                let x = (prev.map_or(w.x0, |p| p.x1) + w.x0) / 2.0;
                stitches.push((run.3 + x - run.0, Point::new(x, y), 0.0));
                run.1 = w.x1;
            } else {
                if let Some(run) = runs.last() {
                    total = run.3 + run.1 - run.0;
                    stitches.push((total, Point::new(w.x0, y), 0.0));
                }
                runs.push((w.x0, w.x1, y, total));
            }
            prev = Some(w);
        }
        if let Some(run) = runs.last() {
            total = run.3 + run.1 - run.0;
        }
        let mut th = Thread {
            at,
            runs,
            stitches,
            total,
            ink,
            still,
        };
        for i in 0..th.stitches.len() {
            th.stitches[i].2 = th.passes(th.stitches[i].0);
        }
        th
    }

    fn until(&self) -> f64 {
        self.at + THREAD_IN + PULL_SECS + 0.4
    }

    /// How much of the thread is left `e` seconds after the cancel.
    fn left(&self, e: f64) -> f32 {
        if self.still {
            return 0.0;
        }
        self.total * (1.0 - ease_in_out((e - THREAD_IN) / PULL_SECS))
    }

    /// When the pulling end passes `s` along the thread, from the cancel.
    fn passes(&self, s: f32) -> f64 {
        let (mut lo, mut hi) = (0.0, THREAD_IN + PULL_SECS);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if self.left(mid) > s {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let e = t - self.at;
        if e < 0.0 || self.runs.is_empty() {
            return;
        }
        let a = if self.still {
            1.0
        } else {
            (e / THREAD_IN).clamp(0.0, 1.0) as f32
        };
        let left = self.left(e);
        let style = StrokeStyle::solid(1.25, self.ink.mark.fade(a));
        let mut head = None;
        for &(x0, x1, y, s0) in &self.runs {
            let x_end = x0 + (left - s0).clamp(0.0, x1 - x0);
            if left >= s0 {
                head = Some(Point::new(r.x + x_end, r.y + y));
            }
            if x_end > x0 {
                p.stroke(&make::underline(r.x + x0, r.x + x_end, r.y + y), &style);
            }
        }
        for &(s, at, passed) in &self.stitches {
            let pt = Point::new(r.x + at.x, r.y + at.y);
            let age = if self.still { 1.0 } else { e - passed };
            if left > s {
                // A stitch still holding: a short cross tick.
                p.shape(ShapeInstance::line(
                    Point::new(pt.x, pt.y - 3.0),
                    Point::new(pt.x, pt.y + 3.0),
                    1.0,
                    Paint::solid(self.ink.mark.fade(a)),
                ));
            } else {
                // Let go: it closes into a dot that stays, fading.
                let k = (age / 0.3).clamp(0.0, 1.0) as f32;
                p.shape(ShapeInstance::circle(
                    pt,
                    1.1,
                    Paint::solid(self.ink.mark.fade(0.9 - 0.6 * k)),
                ));
            }
        }
        if let Some(h) = head {
            p.shape(ShapeInstance::circle(
                h,
                2.2,
                Paint::solid(self.ink.mark.fade(a)),
            ));
        }
    }
}

// MO-50 ---------------------------------------------------------------------

/// When each word rewinds, from the cancel, and how long the whole rewind
/// takes. `times` are the words' spoken start and end in reading order.
/// The newest word goes first; MO-05's spoken offsets replay the words'
/// ends backwards at `1 / REWIND_RATE` of the spoken time, the whole run
/// between `REWIND_MIN` and `REWIND_MAX`, filling in words without a time
/// from their neighbours and spacing them evenly with none known. Each
/// word shrinks until the next older one starts.
fn rewind_schedule(times: &[Option<(f32, f32)>]) -> (Vec<(f32, f32)>, f32) {
    let n = times.len();
    if n == 0 {
        return (Vec::new(), REWIND_MIN);
    }
    // In rewind order, newest first, as time running backwards.
    let back: Vec<Option<f64>> = times
        .iter()
        .rev()
        .map(|t| t.map(|(s, e)| -(e.max(s) as f64)))
        .collect();
    let fit = SpokenFit {
        scale: 1.0 / REWIND_RATE as f64,
        min: REWIND_MIN as f64,
        max: REWIND_MAX as f64,
    };
    let offsets = spoken_offsets(&back, fit);
    let last = offsets.last().copied().unwrap_or(0.0) as f32;
    // The oldest word takes about one share of the run, so it is seen to go.
    let share = if n > 1 {
        last / (n - 1) as f32
    } else {
        REWIND_MIN
    };
    let total = last + share.max(0.03);
    let mut windows = vec![(0.0, total); n];
    for (k, &from) in offsets.iter().enumerate() {
        let to = offsets.get(k + 1).map_or(total, |&o| o as f32);
        let from = from as f32;
        windows[n - 1 - k] = (from, to.max(from + 0.02).min(total.max(from + 0.02)));
    }
    (windows, total)
}

/// The rewind cancel: the look's own words are hidden and these are drawn
/// in their place, going back into the caret.
struct Rewind {
    at: f64,
    laid: Arc<Laid>,
    windows: Vec<(f32, f32)>,
    /// For each word, until when the caret rests at it: when the next
    /// older word on screen starts to go, or never for the first word. A
    /// look that draws each line in its own canvas so shows one caret.
    holds: Vec<f32>,
    total: f32,
    ink: Ink,
    still: bool,
}

impl Rewind {
    fn until(&self) -> f64 {
        self.at + self.total as f64 + 0.3
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        if t < self.at {
            return;
        }
        let e = if self.still {
            10.0
        } else {
            (t - self.at) as f32
        };
        let laid = &*self.laid;
        let style = laid.style;
        let o = Point::new(r.x, r.y);
        // The caret sits at the right edge of the word being taken back,
        // then at its start until the next older word goes, and ends where
        // the first word began.
        let mut caret = None;
        for (i, w) in laid.words.iter().enumerate() {
            let Some(&(b, end)) = self.windows.get(i) else {
                continue;
            };
            let k = ((e - b) / (end - b).max(1e-3)).clamp(0.0, 1.0);
            let s = 1.0 - ease_in_out(k as f64);
            if e >= b && e < self.holds.get(i).copied().unwrap_or(f32::INFINITY) {
                caret = Some((w.x0 + (w.x1 - w.x0) * s, w.baseline));
            }
            if k >= 1.0 {
                continue;
            }
            let ink = self.ink.text.fade(1.0 - 0.6 * k);
            if k <= 0.0 {
                for g in &laid.text.glyphs[w.first..w.end] {
                    p.glyph(glyph(g, o, &style, ink));
                }
            } else {
                let pivot = Point::new(o.x + w.x0, o.y + w.baseline);
                let m = Affine::translate(-pivot.x, -pivot.y)
                    .then(&Affine::scale(s))
                    .then(&Affine::translate(pivot.x, pivot.y));
                p.layer(m, 1.0, |p| {
                    for g in &laid.text.glyphs[w.first..w.end] {
                        p.glyph(glyph(g, o, &style, ink));
                    }
                });
            }
        }
        let Some((x, base)) = caret else {
            return;
        };
        // Under reduced motion the caret stays where the first word began.
        let fade = if self.still {
            1.0
        } else {
            1.0 - ((e - self.total) / 0.2).clamp(0.0, 1.0)
        };
        if fade > 0.0 {
            p.shape(ShapeInstance::line(
                Point::new(o.x + x + 1.0, o.y + base + 2.0),
                Point::new(o.x + x + 1.0, o.y + base - style.size * 0.8),
                1.5,
                Paint::solid(self.ink.mark.fade(fade)),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scrub_goes_back_over_the_spoken_span_and_stops() {
        let s = Scrub {
            at: 10.0,
            total: 0.3,
            span: 1.2,
            on: true,
            still: false,
        };
        assert_eq!(s.map(9.0), 9.0);
        assert!((s.map(10.15) - 9.4).abs() < 1e-6, "half way back");
        assert!((s.map(10.3) - 8.8).abs() < 1e-6);
        assert!((s.map(12.0) - 8.8).abs() < 1e-6, "then it holds");
        assert_eq!(Scrub::default().map(12.0), 12.0);
    }

    #[test]
    fn loading_and_listening_keep_their_shared_letters_in_order() {
        let pairs = lcs_pairs("Loading", "Listening");
        // L, i, n, g: four shared letters, none crossing another.
        assert_eq!(pairs.len(), 4);
        assert_eq!(pairs[0], (0, 0));
        assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1));
        let a: Vec<char> = "Loading".chars().collect();
        let b: Vec<char> = "Listening".chars().collect();
        assert!(pairs.iter().all(|&(i, j)| a[i] == b[j]));
    }

    #[test]
    fn segments_trade_places_and_keep_three_slots() {
        for k in 0..60 {
            let e = k as f64 * 0.1;
            let s = trade_places(e, TRADE_SECS);
            let mut sorted = s;
            sorted.sort_by(f32::total_cmp);
            // Between trades the three slots are 0, 1 and 2.
            if (e / TRADE_SECS).fract() > 0.75 {
                assert_eq!(sorted.map(f32::round), [0.0, 1.0, 2.0], "e {e}: {s:?}");
            }
            assert!(s.iter().all(|x| (-0.01..=2.01).contains(x)));
        }
        // The first trade swaps the left pair.
        let after = trade_places(TRADE_SECS * 0.9, TRADE_SECS);
        assert_eq!(after.map(f32::round), [1.0, 0.0, 2.0]);
    }

    #[test]
    fn rewind_replays_spoken_time_backwards_four_times_faster() {
        let times = [Some((0.0, 0.3)), Some((0.4, 0.8)), Some((1.0, 1.2))];
        let (w, total) = rewind_schedule(&times);
        // The ends are 0.4 s and 0.5 s apart; at a quarter, 0.1 s and 0.125 s.
        assert_eq!(w[2].0, 0.0, "the newest word goes first");
        assert!((w[1].0 - 0.1).abs() < 1e-5, "{w:?}");
        assert!((w[0].0 - 0.225).abs() < 1e-5, "{w:?}");
        // Each word goes until the next older one starts; the oldest last.
        assert_eq!(w[2].1, w[1].0);
        assert_eq!(w[1].1, w[0].0);
        assert!((w[0].1 - total).abs() < 1e-6);
        assert!(total > 0.225 && total < 0.4);
    }

    #[test]
    fn a_long_rewind_is_clamped_to_half_a_second() {
        let times: Vec<_> = (0..40)
            .map(|i| Some((i as f32 * 0.3, i as f32 * 0.3 + 0.25)))
            .collect();
        let (w, total) = rewind_schedule(&times);
        assert!(
            (w[0].0 - REWIND_MAX).abs() < 1e-5,
            "the oldest word starts at 500 ms"
        );
        assert!(total < REWIND_MAX + 0.05);
        assert!(w
            .iter()
            .all(|&(a, b)| a >= 0.0 && b <= total + 1e-6 && a < b));
    }

    #[test]
    fn without_word_times_the_rewind_goes_in_arrival_order() {
        let times = vec![None, Some((0.0, 0.2)), None, None];
        let (w, total) = rewind_schedule(&times);
        assert!((REWIND_MIN..=REWIND_MAX).contains(&total));
        assert_eq!(w[3].0, 0.0);
        assert!(w[0].0 > w[1].0 && w[1].0 > w[2].0 && w[2].0 > w[3].0);
        assert!(w.iter().all(|&(a, b)| b <= total + 1e-6 && a < b));
    }

    fn laid_line() -> Laid {
        let style = TextStyle::new(SANS, 16.0, 20.0);
        let wb = |x0: f32, x1: f32, baseline: f32| WordBox {
            id: 0,
            x0,
            x1,
            baseline,
            first: 0,
            end: 0,
        };
        Laid {
            text: ShapedText::default(),
            words: vec![
                wb(0.0, 30.0, 15.0),
                wb(36.0, 60.0, 15.0),
                wb(66.0, 100.0, 15.0),
                wb(0.0, 40.0, 35.0),
            ],
            style,
        }
    }

    #[test]
    fn the_thread_lets_go_of_its_stitches_from_the_newest_word_back() {
        let ink = Ink {
            text: Color::WHITE,
            mark: Color::WHITE,
            faint: Color::WHITE,
        };
        let th = Thread::new(&laid_line(), 1.0, ink, false);
        assert_eq!(th.runs.len(), 2, "one run per line");
        assert_eq!(th.total, 140.0);
        // Two stitches between words on the first line, one at the wrap.
        assert_eq!(th.stitches.len(), 3);
        let passed: Vec<f64> = th.stitches.iter().map(|s| s.2).collect();
        assert!(passed[2] < passed[1] && passed[1] < passed[0], "{passed:?}");
        assert!(passed
            .iter()
            .all(|&p| p > THREAD_IN && p < THREAD_IN + PULL_SECS));
        assert_eq!(th.left(0.0), 140.0);
        assert!(th.left(THREAD_IN + PULL_SECS) < 1e-3);
    }
}
