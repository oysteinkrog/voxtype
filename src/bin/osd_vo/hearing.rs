//! The popup hears before it reads (MO-11, MO-12, MO-30), from
//! `vo::scene::hearing`: the voice reaches the popup 100 to 400 ms before
//! the words do, so it reacts to the voice first.
//!
//! - `pre-echo` (MO-11): while you speak and no word has come, faint blobs
//!   grow along the baseline where the next word goes, one per onset, as
//!   wide as the sound lasted. When the words come each blob turns into its
//!   word's box and fades as the letters show.
//! - `pause-marks` (MO-30): after 600 ms of quiet a 1 px tick per 400 ms
//!   along the baseline; at 1.5 s they gather into a hollow comma right after
//!   the last word, and a hollow full stop past 2.6 s. Speech squeezes the
//!   ticks into the space; a gathered mark stays. The final pass fills a mark
//!   the final text agrees with and removes the rest.
//! - `exhale` (MO-12): after 3 s of quiet everything but the text rests over
//!   1.2 s (the wave goes flat, the frame thins, the clock greys, and a
//!   bookmark in the margin marks the line where speech stopped). The next
//!   onset, before its word exists, snaps it all back in one fast spring.
//!
//! Each is a motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`). A look
//! turns on the ones in its set (`defaults.rs`; swiss has `pause-marks`);
//! `no-<name>` switches one off there.
//! A look takes part by passing its text column through
//! [`Popup::with_hearing`] and its chrome through the `exhale_*` helpers.
//!
//! The audio comes from the signal bus when the daemon sends voice
//! features, else from the levels, so the moments also show in the demo.

use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vo::scene::hearing::{self, Hearing, HearingLook, HearingTune, Mark, Pause};
use vo_core::words::WordId;

pub(crate) const PRE_ECHO: &str = "pre-echo";
pub(crate) const PAUSE_MARKS: &str = "pause-marks";
pub(crate) const EXHALE: &str = "exhale";

/// Words come without times: what the engine hears takes about this long to
/// become a word.
const UNTIMED_LAG: f64 = 0.15;
/// The wave keeps this much of its height when exhaled.
const FLAT_WAVE: f32 = 0.08;
/// A filled mark shows this long after the final text, then leaves.
const FILLED_SECS: f64 = 0.6;

/// A pause mark that formed after a word.
#[derive(Clone, Copy, Debug)]
struct Placed {
    /// Pause number, for element keys.
    n: u64,
    /// The word the mark follows.
    after: WordId,
    mark: Mark,
}

/// What the hearing features keep between messages.
pub(crate) struct State {
    hearing: Hearing,
    /// The levels path: frames fed so far, against `Popup::received`.
    fed: u64,
    /// The source in use: the signal bus once it has frames.
    bus: bool,
    /// Words in the model at the last sync.
    words: usize,
    /// Which word each read blob turns into.
    blob_word: HashMap<u64, WordId>,
    /// Pause number and when its quiet began.
    pause: Option<(u64, f64)>,
    next_pause: u64,
    placed: Vec<Placed>,
    /// When speech stopped before the current exhale: the word it stopped
    /// after, for the bookmark.
    stopped_after: Option<WordId>,
    /// The shown words laid out at the last view, kept while the words, the
    /// style and the width stay the same: on room tone every view would lay
    /// out the same text again.
    laid: RefCell<Option<Rc<Laid>>>,
}

impl Default for State {
    fn default() -> State {
        State {
            hearing: Hearing::default(),
            fed: 0,
            bus: false,
            words: 0,
            blob_word: HashMap::new(),
            pause: None,
            next_pause: 0,
            placed: Vec::new(),
            stopped_after: None,
            laid: RefCell::new(None),
        }
    }
}

/// Levels on the signal bus run from -60 dBFS (0) to 0 dBFS (1): voice is
/// above about -45 dBFS, quiet below about -51.
fn bus_tune() -> HearingTune {
    HearingTune {
        voice: 0.25,
        quiet: 0.15,
        rise: 0.12,
        ..HearingTune::default()
    }
}

impl Popup {
    /// Whether the hearing feature `name` is on in this look.
    /// The look's own set (`defaults.rs`) decides, so `[osd] motion =
    /// "none"` turns them all off.
    pub(crate) fn hearing_on(&self, name: &str) -> bool {
        self.look.on(name)
    }

    fn hearing_any(&self) -> bool {
        [PRE_ECHO, PAUSE_MARKS, EXHALE]
            .iter()
            .any(|n| self.hearing_on(n))
    }

    fn hearing_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// Feeds the new audio and words to the hearing state after a message.
    pub(crate) fn sync_hearing(&mut self) {
        if !self.hearing_any() {
            return;
        }
        let x = &mut self.hearing;
        // A new recording: the levels start again.
        if self.received < x.fed {
            *x = State::default();
        }
        let bus = self.signals.get();
        if !x.bus && !bus.is_empty() {
            x.bus = true;
            x.hearing = Hearing::new(bus_tune());
        }
        if x.bus {
            let after = x.hearing.clock().unwrap_or(f64::NEG_INFINITY);
            for (_, f) in bus.frames().filter(|(_, f)| f.clock > after) {
                let level = ((f.rms_dbfs + 60.0) / 60.0).clamp(0.0, 1.0);
                x.hearing.hear(f.clock, level, Some(f.onset));
            }
        } else {
            let new = (self.received - x.fed) as usize;
            let n = self.levels.len();
            for (i, &l) in self.levels.iter().skip(n.saturating_sub(new)).enumerate() {
                let index = self.received - new.min(n) as u64 + i as u64;
                x.hearing
                    .hear(index as f64 / FRAME_HZ as f64, l as f32, None);
            }
        }
        x.fed = self.received;

        // The words: blobs before the newest word's end are read.
        let words = self.model.words();
        let new_words = words.len().saturating_sub(x.words);
        let until = words
            .iter()
            .filter_map(|w| self.model.info(w.id).and_then(|i| i.end))
            .fold(None, |m: Option<f32>, e| Some(m.map_or(e, |m| m.max(e))));
        let before: Vec<u64> = read_ids(&x.hearing);
        match until {
            Some(e) => x.hearing.read_until(e as f64, new_words),
            None if new_words > 0 => x.hearing.read_untimed(new_words, UNTIMED_LAG),
            None => 0,
        };
        for b in x.hearing.blobs() {
            if b.read_at.is_some() && !before.contains(&b.id) {
                if let Some(w) = x.words.checked_add(b.word).and_then(|i| words.get(i)) {
                    x.blob_word.insert(b.id, w.id);
                }
            }
        }
        let live: Vec<u64> = x.hearing.blobs().iter().map(|b| b.id).collect();
        x.blob_word.retain(|id, _| live.contains(id));
        x.words = words.len();

        // The pause: number each one, and place its mark after the last word.
        match (x.hearing.pause(), x.hearing.quiet_from()) {
            (Some(p), Some(from)) => {
                if x.pause.is_none_or(|(_, f)| f != from) {
                    x.pause = Some((x.next_pause, from));
                    x.next_pause += 1;
                    x.stopped_after = words.last().map(|w| w.id);
                }
                let n = x.pause.map_or(0, |p| p.0);
                if let (Some(mark), Some(after)) = (p.mark, words.last()) {
                    let ends = after.text.ends_with(|c: char| c.is_ascii_punctuation());
                    match x.placed.iter_mut().find(|m| m.n == n) {
                        Some(m) => m.mark = mark,
                        None if !ends => x.placed.push(Placed {
                            n,
                            after: after.id,
                            mark,
                        }),
                        None => {}
                    }
                }
            }
            _ => x.pause = None,
        }
        let ids: Vec<WordId> = words.iter().map(|w| w.id).collect();
        x.placed.retain(|m| ids.contains(&m.after));
    }

    /// How far the chrome has exhaled, 0 awake to 1 at rest; 0 when the
    /// feature is off or the dictation is not listening.
    pub(crate) fn exhale(&self) -> f32 {
        if !self.hearing_on(EXHALE) || self.kit_mode() != kit::Mode::Listening {
            return 0.0;
        }
        self.hearing.hearing.exhale(self.hearing_reduced())
    }

    /// `c` moved toward `to` as the chrome exhales.
    pub(crate) fn exhale_color(&self, c: Color, to: Color) -> Color {
        kit::mix(c, to, self.exhale())
    }

    /// A stroke `width` thinned to two thirds as the chrome exhales.
    pub(crate) fn exhale_width(&self, width: f32) -> f32 {
        width * (1.0 - self.exhale() / 3.0)
    }

    /// The wave's levels flattened as the chrome exhales.
    pub(crate) fn exhale_levels(&self, levels: &Arc<[f32]>) -> Arc<[f32]> {
        let e = self.exhale();
        if e <= 0.0 {
            return levels.clone();
        }
        let k = 1.0 - (1.0 - FLAT_WAVE) * e;
        levels.iter().map(|l| l * k).collect()
    }

    /// `e` springs back from the exhale in one fast spring on `props`, when
    /// the feature is on.
    pub(crate) fn exhale_motion(&self, mut e: Element<Msg>, props: &[Prop]) -> Element<Msg> {
        if !self.hearing_on(EXHALE) {
            return e;
        }
        for &p in props {
            e = e.motion(p, hearing::SNAP_BACK);
        }
        e
    }

    /// How the chrome moves: one fast spring back from the exhale when the
    /// feature is on, else at once.
    pub(crate) fn exhale_snap(&self) -> Motion {
        if self.hearing_on(EXHALE) {
            Motion::Spring(hearing::SNAP_BACK)
        } else {
            Motion::Snap
        }
    }

    /// The text column `text`, its words laid out in `style` at `width`, with
    /// the pre-echo blobs, the pause marks and the exhale bookmark over it,
    /// in `ink` (the text colour) and `accent`.
    pub(crate) fn with_hearing(
        &self,
        mut text: Element<Msg>,
        style: &TextStyle,
        width: f32,
        ink: Color,
        accent: Color,
    ) -> Element<Msg> {
        if !self.hearing_any() {
            return text;
        }
        // Behind the words: a blob turning into its word's box sits under the letters.
        for e in self.hearing_marks(style, width, ink, accent) {
            text = text.child_behind(e);
        }
        text
    }

    fn hearing_marks(
        &self,
        style: &TextStyle,
        width: f32,
        ink: Color,
        accent: Color,
    ) -> Vec<Element<Msg>> {
        let mode = self.kit_mode();
        let listening = mode == kit::Mode::Listening;
        let finished = matches!(mode, kit::Mode::Finishing | kit::Mode::Done);
        if !listening && !finished {
            return Vec::new();
        }
        let x = &self.hearing;
        let look = HearingLook::for_text(style.size, ink);
        let laid = self.laid(style, width);
        let mut out = Vec::new();

        if self.hearing_on(PRE_ECHO) && listening {
            let (caret, room) = laid.caret(width);
            // Each read blob goes to the box of the word it was handed to;
            // under reduced motion it only fades where it is.
            let travel = !self.hearing_reduced();
            out.extend(hearing::pre_echo(&x.hearing, &look, caret, room, |b| {
                x.blob_word
                    .get(&b.id)
                    .filter(|_| travel)
                    .and_then(|id| laid.word_box(*id))
            }));
        }

        if self.hearing_on(PAUSE_MARKS) {
            let marks = HearingLook {
                ink: accent,
                ..look
            };
            let start = out.len();
            let final_at = self.extra.final_at;
            for m in &x.placed {
                let Some(end) = laid.word_end(m.after) else {
                    continue;
                };
                let word = self.shown.iter().find(|w| w.id == m.after);
                let agrees = word.is_some_and(|w| w.text.ends_with(m.mark.char()));
                let filled = final_at.is_some() && agrees;
                if final_at.is_some_and(|f| !agrees || self.extra.now - f > FILLED_SECS) {
                    continue;
                }
                // Once gathered, the mark alone: no ticks.
                let p = Pause {
                    quiet: 0.0,
                    ticks: 0,
                    mark: Some(m.mark),
                };
                out.extend(hearing::pause_marks(m.n, &p, &marks, end, filled));
            }
            if let (true, Some(p), Some((n, _))) = (listening, x.hearing.pause(), x.pause) {
                if p.mark.is_none() && p.ticks > 0 {
                    let (caret, _) = laid.caret(width);
                    out.extend(hearing::pause_marks(n, &p, &marks, caret, false));
                }
            }
            if self.hearing_reduced() {
                // Under reduced motion a tick or mark only fades in and out
                // where it stands: no tick grows up, no mark draws itself in.
                let still = Transition::fade();
                for e in &mut out[start..] {
                    let taken = std::mem::replace(e, Element::row("pause-taken"));
                    *e = taken
                        .presence(still, still)
                        .motion(Prop::Position, Motion::Snap)
                        .motion(Prop::Size, Motion::Snap)
                        .motion(Prop::Trim, Motion::Snap);
                }
            }
        }

        let e = self.exhale();
        if e > 0.0 {
            if let Some(y) = x.stopped_after.and_then(|w| laid.word_end(w)).map(|p| p.1) {
                // A short bar in the left margin beside the line where speech stopped.
                let h = style.size * 0.7;
                let bar = Element::shape("exhale-bookmark", ShapeKind::RRect)
                    .fill(accent)
                    .opacity(0.8 * e)
                    .motion(Prop::Opacity, hearing::SNAP_BACK)
                    .presence(
                        Transition::fade().opacity(0.0),
                        Transition::fade().opacity(0.0),
                    );
                out.push(place(bar, vo::scene::geom::Rect::new(-7.0, y - h, 2.0, h)));
            }
        }
        out
    }
}

impl Popup {
    /// The shown words laid out in `style` at `width`, from the last view
    /// when nothing it reads has changed.
    fn laid(&self, style: &TextStyle, width: f32) -> Rc<Laid> {
        let mut kept = self.hearing.laid.borrow_mut();
        if let Some(l) = kept
            .as_ref()
            .filter(|l| l.is_for(&self.shown, style, width))
        {
            return l.clone();
        }
        let l = Rc::new(Laid::new(self, style, width));
        *kept = Some(l.clone());
        l
    }
}

fn read_ids(h: &Hearing) -> Vec<u64> {
    h.blobs()
        .iter()
        .filter(|b| b.read_at.is_some())
        .map(|b| b.id)
        .collect()
}

/// `e` placed absolutely at `r` in its parent.
fn place(e: Element<Msg>, r: vo::scene::geom::Rect) -> Element<Msg> {
    use vo::scene::taffy;
    use vo::scene::taffy::prelude::{auto, length};
    e.size(r.w.max(1.0), r.h.max(1.0)).layout(|s| {
        s.position = taffy::Position::Absolute;
        s.inset = taffy::Rect {
            left: length(r.x),
            top: length(r.y),
            right: auto(),
            bottom: auto(),
        };
    })
}

/// The words on screen laid out as the live text lays them out, for
/// placing marks beside them.
struct Laid {
    text: vo::scene::text::ShapedText,
    /// Each shown word's id and byte range in the joined text.
    ranges: Vec<(WordId, std::ops::Range<usize>)>,
    space: f32,
    line_height: f32,
    first_baseline: f32,
    /// What it was laid out from: the words' texts, the style and the width.
    words: Vec<String>,
    style: TextStyle,
    width: f32,
}

impl Laid {
    fn new(p: &Popup, style: &TextStyle, width: f32) -> Laid {
        let mut joined = String::new();
        let mut ranges = Vec::new();
        for w in &p.shown {
            if !joined.is_empty() {
                joined.push(' ');
            }
            let start = joined.len();
            joined.push_str(&w.text);
            ranges.push((w.id, start..joined.len()));
        }
        let text = p.measure.layout(&joined, style, Some(width));
        let space = p.measure.layout("a a", style, None).size.w
            - 2.0 * p.measure.layout("a", style, None).size.w;
        let probe = p.measure.layout("Hg", style, None);
        let first_baseline = probe.lines.first().map_or(style.size, |l| l.baseline);
        Laid {
            text,
            ranges,
            space: space.max(style.size * 0.25),
            line_height: style.line_height,
            first_baseline,
            words: p.shown.iter().map(|w| w.text.clone()).collect(),
            style: *style,
            width,
        }
    }

    /// Whether it was laid out from these words, style and width.
    fn is_for(&self, shown: &[Word], style: &TextStyle, width: f32) -> bool {
        self.style == *style
            && self.width == width
            && self.ranges.len() == shown.len()
            && self
                .ranges
                .iter()
                .zip(&self.words)
                .zip(shown)
                .all(|(((id, _), text), w)| *id == w.id && *text == w.text)
    }

    fn glyphs(&self, id: WordId) -> impl Iterator<Item = &vo::scene::text::PlacedGlyph> + '_ {
        let range = self
            .ranges
            .iter()
            .find(|(w, _)| *w == id)
            .map(|(_, r)| r.clone())
            .unwrap_or(0..0);
        self.text
            .glyphs
            .iter()
            .filter(move |g| range.contains(&(g.cluster as usize)))
    }

    /// The right end of a word's ink on its baseline.
    fn word_end(&self, id: WordId) -> Option<(f32, f32)> {
        self.glyphs(id)
            .max_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .map(|g| (g.pos.x + g.advance, g.pos.y))
    }

    /// A word's box: its ink across, a line tall around its baseline.
    fn word_box(&self, id: WordId) -> Option<vo::scene::geom::Rect> {
        let (mut x0, mut x1, mut y) = (f32::INFINITY, f32::NEG_INFINITY, 0.0);
        for g in self.glyphs(id) {
            x0 = x0.min(g.pos.x);
            x1 = x1.max(g.pos.x + g.advance);
            y = g.pos.y;
        }
        (x0 < x1).then(|| {
            let h = self.line_height;
            vo::scene::geom::Rect::new(x0 - 3.0, y - h * 0.72, x1 - x0 + 6.0, h * 0.9)
        })
    }

    /// Where the next word starts on the reading line, and the room left there.
    fn caret(&self, width: f32) -> ((f32, f32), f32) {
        match self.ranges.last().and_then(|(id, _)| self.word_end(*id)) {
            Some((x, y)) => {
                let x = x + self.space;
                ((x, y), (width - x).max(0.0))
            }
            None => ((0.0, self.first_baseline), width),
        }
    }
}
