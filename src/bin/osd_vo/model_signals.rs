//! The live model's own signals on the live text (ASR-05), from
//! `preview-signals.json` (`voxtype::osd::live::ModelSignals`), each behind a
//! motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`). All are off unless
//! asked for.
//!
//! - `ghost-tail`: the words the model hears in its lookahead (the newest
//!   0.56 s, decoded but not committed) follow the live text at 45 % ink,
//!   where the committed words will land. When a word commits, the real word
//!   takes its place and the ghost fades; when the guess changes, the old
//!   ghost fades out as the new one fades in.
//! - `three-tier-ink`: the tail in two tiers below the committed text's full
//!   ink. A pending word, 3 or more frames from the window edge, is about
//!   99 % right and sits at 70 % ink; an edge guess, closer than that, is
//!   right only 17 to 38 % of the time and sits at 30 %.
//! - `listening-floor`: a thin ribbon just under the text box (below the
//!   descenders), one bar per 80 ms
//!   frame of the last 5.6 s, its height log10(1 - P(blank)) clipped to
//!   [-6, 0]: flat in silence, rising where the model nearly heard a token.
//!   The bars ease to each step's values and slide as new frames come.
//!
//! - `commit-cursor`: a hairline where the committed text ends and a
//!   fainter one where the model has heard to (the end of the tail), with a
//!   faint thread between them on one line: the window the model is still
//!   thinking about, 0.56 s of audio today.
//!
//! - `graded-doubt`: an underline under each word the model was unsure of,
//!   from entropy 0.3 nats (hairline, faint) to 1.0 (2.4 px, strong), until
//!   the final pass. It replaces the type look's on/off doubt line, which
//!   fires only under 60 % confidence, where it is on.
//!
//! - `breathing-stop`: when the tail expects a full stop, it sits after the
//!   text at its probability's ink (60 % at certainty, so it never reads
//!   as typed), fading to each new probability over 0.3 s and pulling
//!   back when the model drops it. The ghost words then leave their own
//!   full stop out.
//!
//! - `pre-echo-letters`: when a frame of the newest step came within
//!   10^-2.5 of a token, the first letters of the model's top guess there
//!   flash at 25 % ink after the text, about one frame before the piece
//!   commits. (Not `pre-echo`, the hearing caret of MO-11.)
//!
//! - `twin-spelling`: when a word commits with its top two guesses less
//!   than 0.2 apart, the runner-up shows small above it for 0.4 s and
//!   fades over 0.8 s: the other word the model nearly wrote.
//!
//! - `birth-burst`: each new word gets a ring that opens out from it over
//!   0.6 s. The ring opens wider when several words came in one step and
//!   is brighter when the frame it started on was a near-certain token.
//!   Nothing shows under reduced motion.
//!
//! - `mind-change`: when a step replaces a word the tail had guessed, the
//!   old word lingers over the new one for 300 ms, fading. Each change in
//!   the last 3 s tints a bar at the text column's right edge, so a model
//!   that keeps changing its mind shows it. Under reduced motion the old
//!   word holds still for the 300 ms.
//!
//! Nothing shows once the recording stops, so the signals end with the
//! dictation and nothing moves at room tone. Under reduced motion the ghosts
//! swap at once (no fades) and the floor jumps between steps: the ink tiers
//! stay.
//!
//! A look takes part by passing its text column through
//! [`Popup::signal_marks`].

use super::*;

use voxtype::osd::live::{ModelSignals, TailToken};

pub(crate) const GHOST_TAIL: &str = "ghost-tail";
pub(crate) const THREE_TIER: &str = "three-tier-ink";
pub(crate) const FLOOR: &str = "listening-floor";
pub(crate) const COMMIT_CURSOR: &str = "commit-cursor";
pub(crate) const GRADED_DOUBT: &str = "graded-doubt";
pub(crate) const BREATHING_STOP: &str = "breathing-stop";
pub(crate) const PRE_ECHO_LETTERS: &str = "pre-echo-letters";
pub(crate) const TWIN_SPELLING: &str = "twin-spelling";
pub(crate) const BIRTH_BURST: &str = "birth-burst";
pub(crate) const MIND_CHANGE: &str = "mind-change";

/// Every model signal feature.
const ALL: [&str; 10] = [
    MIND_CHANGE,
    BIRTH_BURST,
    TWIN_SPELLING,
    PRE_ECHO_LETTERS,
    GHOST_TAIL,
    THREE_TIER,
    FLOOR,
    COMMIT_CURSOR,
    GRADED_DOUBT,
    BREATHING_STOP,
];

/// Ink of a tail word with `ghost-tail` alone.
const GHOST_INK: f32 = 0.45;
/// Ink of a pending tail word (`three-tier-ink`).
const PENDING_INK: f32 = 0.7;
/// Ink of an edge guess (`three-tier-ink`).
const EDGE_INK: f32 = 0.3;
/// A tail token this many frames or more from the window edge is pending.
const PENDING_FRAMES: u32 = 3;
/// The floor's range in log10(1 - P(blank)).
const FLOOR_MIN: f32 = -6.0;
/// The floor's tallest bar and its resting height, in px.
const FLOOR_PX: f32 = 4.0;
const FLOOR_REST_PX: f32 = 0.5;
/// The commit cursor's hairlines: width, height as a share of the line, and
/// the gap after the last glyph, in px.
const CURSOR_PX: f32 = 1.5;
const CURSOR_HEIGHT: f32 = 0.7;
const CURSOR_GAP: f32 = 2.0;
/// Graded doubt: a word's entropy (nats) from which it gets an underline,
/// and at which the underline is at its heaviest.
const DOUBT_FROM: f32 = 0.3;
const DOUBT_FULL: f32 = 1.0;
/// The underline's weight at the lightest and heaviest, in px.
const DOUBT_MIN_PX: f32 = 0.6;
const DOUBT_MAX_PX: f32 = 2.4;
/// The breathing full stop: its ink at probability 1, and how long it
/// takes to reach a new probability.
const STOP_INK: f32 = 0.6;
const STOP_FADE_SECS: f64 = 0.3;
/// Pre-echo letters: the level a frame must pass, the frames of one step
/// (0.56 s of 80 ms frames), the letters shown and their ink.
const ECHO_GATE: f32 = -2.5;
const STEP_FRAMES: usize = 7;
const ECHO_LETTERS: usize = 2;
const ECHO_INK: f32 = 0.25;
/// Twin spelling: the margin under which the runner-up shows, how long it
/// holds and fades, its ink at birth and its size against the text.
const TWIN_MARGIN: f32 = 0.2;
const TWIN_HOLD: f64 = 0.4;
const TWIN_FADE: f64 = 0.8;
const TWIN_INK: f32 = 0.55;
const TWIN_SCALE: f32 = 0.55;
/// Birth burst: how long a ring lasts, how far it opens per word born in
/// the same step, the most words counted, and its ink at birth from a
/// silent frame (level -3 or under) to a certain one (level 0).
const BURST_SECS: f64 = 0.6;
const BURST_PX: f32 = 4.0;
const BURST_MAX_WORDS: usize = 4;
const BURST_INK: (f32, f32) = (0.25, 0.7);
/// Words that came within this of each other came in one step.
const BURST_STEP_SECS: f64 = 0.3;
/// Mind change: how long a replaced word lingers and its ink at first, how
/// long a change tints the edge, the tint per change and at most, and the
/// edge bar's width.
const CHANGE_SECS: f64 = 0.3;
const CHANGE_INK: f32 = 0.5;
const FLICKER_SECS: f64 = 3.0;
const FLICKER_INK: f32 = 0.2;
const FLICKER_MAX: f32 = 0.7;
const EDGE_BAR_PX: f32 = 2.0;

/// What the features keep between model steps.
#[derive(Default)]
pub(crate) struct State {
    /// The words of the last step's text and tail, and how many of them
    /// were committed.
    last: Vec<String>,
    last_committed: usize,
    /// Replaced words: their place in the dictation, the old text and when.
    echoes: Vec<(usize, String, f64)>,
    /// When each recent change came.
    flickers: Vec<f64>,
}

/// A step's words for comparing places: split on spaces, without the
/// punctuation the tail adds and drops at its end.
fn place_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.trim_end_matches(['.', ',', '?', '!']).to_string())
        .collect()
}

/// The places where `new` has another word than `old` had, among the places
/// `old` had only guessed (from `committed` on). A tail that grows or
/// drops words changes no place.
fn changed_places(old: &[String], committed: usize, new: &[String]) -> Vec<usize> {
    (committed..old.len().min(new.len()))
        .filter(|&p| old[p] != new[p])
        .collect()
}

/// One word of the tail.
#[derive(Debug, PartialEq)]
struct TailWord {
    text: String,
    /// Frames between its newest token and the window edge.
    edge: u32,
    /// It continues the last committed word ("act" + "ivity").
    joins: bool,
}

/// Groups the tail's tokens into words. A token that starts with a space
/// starts a word; a first token without one continues the last committed
/// word, when there is one.
fn tail_words(tail: &[TailToken], has_committed: bool) -> Vec<TailWord> {
    let mut words: Vec<TailWord> = Vec::new();
    for (i, t) in tail.iter().enumerate() {
        let starts = t.text.starts_with(' ');
        let piece = t.text.trim_start();
        if piece.is_empty() {
            continue;
        }
        match words.last_mut() {
            Some(w) if !starts => {
                w.text.push_str(piece);
                w.edge = w.edge.min(t.edge);
            }
            _ => words.push(TailWord {
                text: piece.to_string(),
                edge: t.edge,
                joins: i == 0 && !starts && has_committed,
            }),
        }
    }
    words
}

/// The live text and the tail laid out together, as the look lays out its
/// live text: where each tail word sits and where the text ends.
struct TailLayout {
    tail: Vec<TailWord>,
    /// Top left of each tail word's first glyph, or None when it has no room.
    at: Vec<Option<(f32, f32)>>,
    /// Where the next glyph after the committed text would go (x, line top).
    committed_end: (f32, f32),
    /// The same after the tail.
    tail_end: (f32, f32),
    /// The text's line height and how many lines it has room for.
    line_h: f32,
    /// With `breathing-stop`: the likeliest full stop in the tail, which is
    /// then left out of the tail words.
    stop: Option<f32>,
}

impl Popup {
    /// Notes what a new model step changed, before it is stored.
    pub(crate) fn note_signals(&mut self, s: &ModelSignals, t: f64) {
        if !self.look.on(MIND_CHANGE) {
            return;
        }
        let words = place_words(&s.with_tail);
        let committed = place_words(&self.files.preview).len();
        let st = &mut self.signal_state;
        if s.with_tail.starts_with(self.files.preview.trim()) {
            for p in changed_places(&st.last, st.last_committed, &words) {
                st.echoes.push((p, st.last[p].clone(), t));
                st.flickers.push(t);
            }
        }
        st.echoes.retain(|e| t - e.2 < CHANGE_SECS);
        st.flickers.retain(|&f| t - f < FLICKER_SECS);
        st.last = words;
        st.last_committed = committed;
    }

    fn signals_on(&self) -> bool {
        ALL.iter().any(|f| self.look.on(f))
    }

    fn signals_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// The signals to draw now: while the recording runs and before the
    /// final pass.
    fn live_signals(&self) -> Option<&ModelSignals> {
        let listening = self.files.recording && !self.status().finishing && self.ending().is_none();
        self.model_signals.as_ref().filter(|_| listening)
    }

    /// The look's text column with the model signals its features ask for:
    /// the ghost tail after `words` (the live text as laid out at `width`)
    /// and the listening floor under it. `origin` is where the live text
    /// starts inside `text`; `ink` is the live text's colour.
    pub(crate) fn signal_marks(
        &self,
        text: Element<Msg>,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Element<Msg> {
        if !self.signals_on() {
            return text;
        }
        let mut text = text;
        for e in self.graded_doubt(words, width, origin) {
            text = text.child(e);
        }
        for e in self.twin_spelling(words, width, origin, ink) {
            text = text.child(e);
        }
        if let Some(s) = self.live_signals() {
            let laid = self.tail_layout(s, words, width);
            for e in self.ghosts(laid.as_ref(), words, origin, ink) {
                text = text.child(e);
            }
            for e in self.mind_change(s, words, width, origin, ink) {
                text = text.child(e);
            }
            for e in self.birth_burst(s, words, width, origin) {
                text = text.child(e);
            }
            for e in self.floor(s, width, origin) {
                text = text.child(e);
            }
            for e in self.commit_cursor(laid.as_ref(), origin) {
                text = text.child(e);
            }
            if let Some(e) = self.breathing_stop(laid.as_ref(), origin, ink) {
                text = text.child(e);
            }
            if let Some(e) = self.pre_echo_letters(s, laid.as_ref(), origin, ink) {
                text = text.child(e);
            }
        }
        text
    }

    /// Lays out `words` and the tail of `s` together. None when the tail
    /// does not belong to the text on screen: then one of the two files is
    /// newer than the other.
    fn tail_layout(&self, s: &ModelSignals, words: &[Word], width: f32) -> Option<TailLayout> {
        let committed = self.files.preview.trim();
        if !s.with_tail.starts_with(committed) {
            return None;
        }
        let style = self.look.text_style();
        let line_h = style.line_height;
        let mut full = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let committed_len = full.len();
        let is_stop = |t: &TailToken| t.text.trim() == ".";
        let (stops, kept): (Vec<TailToken>, Vec<TailToken>) = if self.look.on(BREATHING_STOP) {
            s.tail.iter().cloned().partition(is_stop)
        } else {
            (Vec::new(), s.tail.clone())
        };
        let stop = stops.iter().map(|t| t.conf).reduce(f32::max);
        let tail = tail_words(&kept, !words.is_empty());
        let mut ranges = Vec::with_capacity(tail.len());
        for w in &tail {
            if !w.joins && !full.is_empty() {
                full.push(' ');
            }
            let start = full.len();
            full.push_str(&w.text);
            ranges.push(start..full.len());
        }
        let laid = self.measure.layout(&full, &style, Some(width));
        let room = self.lines as f32 * line_h - 0.5;
        let top_of = |y: f32| (y / line_h).floor() * line_h;
        let at = ranges
            .iter()
            .map(|range| {
                let g = laid
                    .glyphs
                    .iter()
                    .find(|g| range.contains(&(g.cluster as usize)))?;
                let top = top_of(g.pos.y);
                // A guess that would open a line the text has no room for
                // yet waits until the text gets there.
                (top < room).then_some((g.pos.x, top))
            })
            .collect();
        // The end of a run of text: after its last glyph.
        let end_of = |len: usize| -> (f32, f32) {
            match laid.glyphs.iter().rfind(|g| (g.cluster as usize) < len) {
                Some(g) => (g.pos.x + g.advance, top_of(g.pos.y)),
                None => (0.0, 0.0),
            }
        };
        // A tail that runs onto a line with no room yet ends, as far as the
        // screen goes, at the right edge of the last line there is.
        let mut tail_end = end_of(full.len());
        if tail_end.1 >= room {
            tail_end = (width - CURSOR_GAP - CURSOR_PX, top_of(room - 1.0));
        }
        Some(TailLayout {
            tail,
            at,
            committed_end: end_of(committed_len),
            tail_end,
            line_h,
            stop,
        })
    }

    /// The live model's detail of the shown word `i` of `words`, from
    /// `preview.json`, when it describes the text on screen.
    fn word_detail(&self, i: usize) -> Option<&voxtype::osd::live::WordDetail> {
        let doc = self.files.live.as_ref()?;
        if doc.text != self.files.preview {
            return None;
        }
        doc.words.get(self.cut + i)?.detail.as_ref()
    }

    /// `graded-doubt`: an underline under each word the model was unsure
    /// of, its weight and ink growing with the word's entropy, until the
    /// final pass. Most words stay plain.
    fn graded_doubt(&self, words: &[Word], width: f32, origin: (f32, f32)) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(GRADED_DOUBT) || self.status().finishing || self.ending().is_some() {
            return Vec::new();
        }
        let style = l.text_style();
        let joined = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(width));
        let spring = if self.signals_reduced() {
            Spring::SNAP
        } else {
            Spring::critical_to_98(0.3)
        };
        let mut out = Vec::new();
        let mut start = 0usize;
        for (i, w) in words.iter().enumerate() {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let Some(d) = self.word_detail(i) else { continue };
            let k = ((d.entropy - DOUBT_FROM) / (DOUBT_FULL - DOUBT_FROM)).clamp(0.0, 1.0);
            if d.entropy < DOUBT_FROM {
                continue;
            }
            let glyphs: Vec<_> = laid
                .glyphs
                .iter()
                .filter(|g| range.contains(&(g.cluster as usize)))
                .collect();
            let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
                continue;
            };
            let weight = DOUBT_MIN_PX + (DOUBT_MAX_PX - DOUBT_MIN_PX) * k;
            let line = Element::shape(format!("doubt-{}", w.id), ShapeKind::RRect)
                .size(last.pos.x + last.advance - first.pos.x, weight)
                .fill(l.accent.fade(0.3 + 0.5 * k))
                .motion(Prop::Size, spring)
                .motion(Prop::Position, Motion::Snap)
                .presence(Transition::fade(), Transition::fade());
            let y = first.pos.y + 0.16 * style.size;
            out.push(kit::at(line, origin.0 + first.pos.x, origin.1 + y));
        }
        out
    }

    /// `twin-spelling`: the runner-up above each newly committed word the
    /// model nearly wrote another way.
    fn twin_spelling(
        &self,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(TWIN_SPELLING) || self.status().finishing || self.ending().is_some() {
            return Vec::new();
        }
        let now = self.extra.now;
        let born = |id| {
            self.tally
                .arrived
                .iter()
                .find(|(i, _)| *i == id)
                .map(|a| now - a.1)
                .filter(|age| (0.0..TWIN_HOLD + TWIN_FADE).contains(age))
        };
        let style = l.text_style();
        let mut small = style;
        small.size *= TWIN_SCALE;
        small.line_height *= TWIN_SCALE;
        let joined = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(width));
        let reduced = self.signals_reduced();
        let mut out = Vec::new();
        let mut start = 0usize;
        for (i, w) in words.iter().enumerate() {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let Some(age) = born(w.id) else { continue };
            let Some(d) = self.word_detail(i) else { continue };
            let Some(twin) = d.runner_up.as_deref().map(str::trim) else {
                continue;
            };
            if d.margin >= TWIN_MARGIN || twin.is_empty() {
                continue;
            }
            let Some(first) = laid
                .glyphs
                .iter()
                .find(|g| range.contains(&(g.cluster as usize)))
            else {
                continue;
            };
            // Under reduced motion the twin holds still, then goes at once.
            let fade = if reduced {
                1.0
            } else {
                1.0 - ((age - TWIN_HOLD) / TWIN_FADE).max(0.0) as f32
            };
            let e = Element::label(format!("twin-{}", w.id), twin.to_string())
                .text_style(small)
                .color(ink)
                .opacity(TWIN_INK * fade);
            // Its baseline sits just above the word's ascenders.
            let y = first.pos.y - style.size * 0.95 - small.line_height * 0.8;
            out.push(kit::at(e, origin.0 + first.pos.x, origin.1 + y));
        }
        out
    }

    /// `mind-change`: replaced words lingering over their successors, and
    /// the edge bar tinted by recent changes.
    fn mind_change(
        &self,
        s: &ModelSignals,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(MIND_CHANGE) {
            return Vec::new();
        }
        let now = self.extra.now;
        let st = &self.signal_state;
        let style = l.text_style();
        let mut out = Vec::new();
        let echoes: Vec<_> = st
            .echoes
            .iter()
            .filter(|e| (0.0..CHANGE_SECS).contains(&(now - e.2)))
            .collect();
        if !echoes.is_empty() {
            // The places from the first word on screen, laid out as the
            // text and its tail are.
            let shown = s.with_tail.split_whitespace().skip(self.cut);
            let mut full = String::new();
            let mut ranges = Vec::new();
            for w in shown {
                if !full.is_empty() {
                    full.push(' ');
                }
                ranges.push(full.len()..full.len() + w.len());
                full.push_str(w);
            }
            let laid = self.measure.layout(&full, &style, Some(width));
            let room = self.lines as f32 * style.line_height;
            let reduced = self.signals_reduced();
            for (p, old, at) in echoes {
                let Some(range) = p.checked_sub(self.cut).and_then(|i| ranges.get(i)) else {
                    continue;
                };
                let Some(g) = laid
                    .glyphs
                    .iter()
                    .find(|g| range.contains(&(g.cluster as usize)))
                else {
                    continue;
                };
                let top = (g.pos.y / style.line_height).floor() * style.line_height;
                if top >= room {
                    continue;
                }
                let age = ((now - at) / CHANGE_SECS) as f32;
                let (fade, rise) = if reduced { (0.7, 0.0) } else { (1.0 - age, 4.0 * age) };
                let e = Element::label(format!("change-{p}-{old}"), old.clone())
                    .text_style(style)
                    .color(ink)
                    .opacity(CHANGE_INK * fade);
                out.push(kit::at(e, origin.0 + g.pos.x, origin.1 + top - rise));
            }
        }
        let n = st.flickers.iter().filter(|&&f| now - f < FLICKER_SECS).count();
        if n > 0 && !words.is_empty() {
            let tint = (FLICKER_INK * n as f32).min(FLICKER_MAX);
            let h = self.lines as f32 * style.line_height;
            let bar = Element::shape("mind-change-edge", ShapeKind::RRect)
                .size(EDGE_BAR_PX, h)
                .radius(EDGE_BAR_PX * 0.5)
                .fill(l.accent.fade(tint))
                .presence(Transition::fade(), Transition::fade());
            // Inside the column: the looks clip what lies past it.
            out.push(kit::at(bar, origin.0 + width - EDGE_BAR_PX, origin.1));
        }
        out
    }

    /// `birth-burst`: a ring opening out from each new word.
    fn birth_burst(
        &self,
        s: &ModelSignals,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(BIRTH_BURST) || self.signals_reduced() {
            return Vec::new();
        }
        let now = self.extra.now;
        let arrived = &self.tally.arrived;
        let fresh: Vec<(u32, f64)> = arrived
            .iter()
            .filter(|(_, at)| (0.0..BURST_SECS).contains(&(now - at)))
            .copied()
            .collect();
        if fresh.is_empty() {
            return Vec::new();
        }
        let style = l.text_style();
        let joined = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(width));
        let doc = self
            .files
            .live
            .as_ref()
            .filter(|d| d.text == self.files.preview);
        let mut out = Vec::new();
        let mut start = 0usize;
        for (i, w) in words.iter().enumerate() {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let Some(&(_, at)) = fresh.iter().find(|(id, _)| *id == w.id) else {
                continue;
            };
            let glyphs: Vec<_> = laid
                .glyphs
                .iter()
                .filter(|g| range.contains(&(g.cluster as usize)))
                .collect();
            let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
                continue;
            };
            let together = arrived
                .iter()
                .filter(|(_, a)| (a - at).abs() < BURST_STEP_SECS)
                .count()
                .clamp(1, BURST_MAX_WORDS);
            // The level of the frame the word started on.
            let level = doc
                .and_then(|d| d.words.get(self.cut + i)?.start)
                .and_then(|t0| {
                    s.frames
                        .iter()
                        .min_by(|a, b| (a.t - t0).abs().total_cmp(&(b.t - t0).abs()))
                })
                .map_or(-3.0, |f| f.level);
            let k = ((level + 3.0) / 3.0).clamp(0.0, 1.0);
            let age = ((now - at) / BURST_SECS) as f32;
            let grow = 2.0 + BURST_PX * together as f32 * age;
            let ink = (BURST_INK.0 + (BURST_INK.1 - BURST_INK.0) * k) * (1.0 - age);
            let top = first.pos.y - style.size * 0.8;
            let h = style.size * 1.0;
            let ring = Element::shape(format!("burst-{}", w.id), ShapeKind::RRect)
                .size(last.pos.x + last.advance - first.pos.x + 2.0 * grow, h + 2.0 * grow)
                .radius(h * 0.5 + grow)
                .stroke(l.accent.fade(ink), 1.5);
            out.push(kit::at(
                ring,
                origin.0 + first.pos.x - grow,
                origin.1 + top - grow,
            ));
        }
        out
    }

    fn ghosts(
        &self,
        laid: Option<&TailLayout>,
        words: &[Word],
        origin: (f32, f32),
        ink: Color,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        let tiers = l.on(THREE_TIER);
        let Some(laid) = laid.filter(|_| l.on(GHOST_TAIL) || tiers) else {
            return Vec::new();
        };
        let style = l.text_style();
        let reduced = self.signals_reduced();
        let presence = if reduced {
            (Transition::NONE, Transition::NONE)
        } else {
            (Transition::fade(), Transition::fade())
        };
        // A ghost's key is the word's place in the dictation and its text:
        // the same guess keeps its element, a new guess at that place
        // fades in over the old one.
        let first = self.cut + words.len();
        let mut out = Vec::new();
        for (i, (w, at)) in laid.tail.iter().zip(&laid.at).enumerate() {
            let Some((x, top)) = *at else { continue };
            let alpha = match (tiers, w.edge >= PENDING_FRAMES) {
                (true, true) => PENDING_INK,
                (true, false) => EDGE_INK,
                (false, _) => GHOST_INK,
            };
            let place = first + i - usize::from(w.joins);
            let ghost = Element::label(format!("ghost-{place}-{}", w.text), w.text.as_str())
                .text_style(style)
                .color(ink)
                .opacity(alpha)
                .motion(
                    Prop::Opacity,
                    if reduced {
                        Spring::SNAP
                    } else {
                        Spring::critical_to_98(0.2)
                    },
                )
                .motion(Prop::Position, Motion::Snap)
                .presence(presence.0, presence.1);
            out.push(kit::at(ghost, origin.0 + x, origin.1 + top));
        }
        out
    }

    /// `commit-cursor`: a hairline where the committed text ends and a
    /// fainter one where the tail ends, what the model has heard to. The gap
    /// is the window it is still thinking about.
    fn commit_cursor(&self, laid: Option<&TailLayout>, origin: (f32, f32)) -> Vec<Element<Msg>> {
        let l = &self.look;
        let Some(laid) = laid.filter(|_| l.on(COMMIT_CURSOR)) else {
            return Vec::new();
        };
        let spring = if self.signals_reduced() {
            Spring::SNAP
        } else {
            Spring::critical_to_98(0.25)
        };
        let h = laid.line_h * CURSOR_HEIGHT;
        let inset = 0.5 * (laid.line_h - h);
        let mark = |key: &str, (x, top): (f32, f32), alpha: f32| {
            let e = Element::shape(key.to_string(), ShapeKind::RRect)
                .size(CURSOR_PX, h)
                .fill(l.accent.fade(alpha))
                .motion(Prop::Position, spring)
                .presence(Transition::fade(), Transition::fade());
            kit::at(e, origin.0 + x + CURSOR_GAP, origin.1 + top + inset)
        };
        let mut out = vec![
            mark("cursor-committed", laid.committed_end, 0.9),
            mark("cursor-heard", laid.tail_end, 0.45),
        ];
        // On one line, a faint thread between the two shows the window.
        let ((x0, t0), (x1, t1)) = (laid.committed_end, laid.tail_end);
        if t0 == t1 && x1 > x0 {
            let e = Element::shape("cursor-window", ShapeKind::RRect)
                .size(x1 - x0, 1.0)
                .fill(l.accent.fade(0.25))
                .motion(Prop::Position, spring)
                .motion(Prop::Size, spring)
                .presence(Transition::fade(), Transition::fade());
            out.push(kit::at(e, origin.0 + x0 + CURSOR_GAP, origin.1 + t0 + inset + h));
        }
        out
    }

    /// `breathing-stop`: the full stop the tail expects, after the text at
    /// its probability's ink. It fades in over 0.3 s and pulls back when the
    /// model drops it.
    fn breathing_stop(
        &self,
        laid: Option<&TailLayout>,
        origin: (f32, f32),
        ink: Color,
    ) -> Option<Element<Msg>> {
        let l = &self.look;
        let laid = laid.filter(|_| l.on(BREATHING_STOP))?;
        if self.files.preview.trim_end().ends_with('.') {
            return None;
        }
        // After the ghost words when they show, else after the committed text.
        let (x, top) = if l.on(GHOST_TAIL) || l.on(THREE_TIER) {
            laid.tail_end
        } else {
            laid.committed_end
        };
        let spring = if self.signals_reduced() {
            Spring::SNAP
        } else {
            Spring::critical_to_98(STOP_FADE_SECS)
        };
        let alpha = laid.stop.unwrap_or(0.0) * STOP_INK;
        let e = Element::label("breath-stop", ".")
            .text_style(l.text_style())
            .color(ink)
            .opacity(alpha)
            .motion(Prop::Opacity, spring)
            .motion(Prop::Position, Motion::Snap)
            .presence(Transition::fade(), Transition::fade());
        Some(kit::at(e, origin.0 + x, origin.1 + top))
    }

    /// `pre-echo-letters`: the first letters of the top guess of the newest
    /// step's latest frame that came close to a token.
    fn pre_echo_letters(
        &self,
        s: &ModelSignals,
        laid: Option<&TailLayout>,
        origin: (f32, f32),
        ink: Color,
    ) -> Option<Element<Msg>> {
        let l = &self.look;
        let laid = laid.filter(|_| l.on(PRE_ECHO_LETTERS))?;
        let newest = s.frames.len().saturating_sub(STEP_FRAMES);
        let guess = s.frames[newest..]
            .iter()
            .rev()
            .filter(|f| f.level > ECHO_GATE)
            .find_map(|f| f.top.first())?;
        let letters: String = guess.text.trim_start().chars().take(ECHO_LETTERS).collect();
        if letters.is_empty() {
            return None;
        }
        let (mut x, top) = if l.on(GHOST_TAIL) || l.on(THREE_TIER) {
            laid.tail_end
        } else {
            laid.committed_end
        };
        // A guess that starts a word starts after a space.
        let style = l.text_style();
        if guess.text.starts_with(' ') && x > 0.0 {
            x += self.measure.layout(" ", &style, None).size.w;
        }
        let presence = if self.signals_reduced() {
            (Transition::NONE, Transition::NONE)
        } else {
            (Transition::fade(), Transition::fade())
        };
        // Each step's echo is its own element, so it flashes in.
        let e = Element::label(format!("pre-echo-{}", s.frames_total), letters)
            .text_style(style)
            .color(ink)
            .opacity(ECHO_INK)
            .presence(presence.0, presence.1);
        Some(kit::at(e, origin.0 + x, origin.1 + top))
    }

    fn floor(&self, s: &ModelSignals, width: f32, origin: (f32, f32)) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(FLOOR) || s.frames.is_empty() {
            return Vec::new();
        }
        let slots = voxtype::osd::live::SIGNALS_KEEP_FRAMES;
        let bar = width / slots as f32;
        // Below the text box, so a bar never meets a descender: the bars hang
        // from 1 px under the last line into the gap above the status row.
        let base = origin.1 + self.lines as f32 * l.text_style().line_height + 1.0 + FLOOR_PX;
        let spring = if self.signals_reduced() {
            Spring::SNAP
        } else {
            Spring::critical_to_98(0.3)
        };
        // The newest frame sits at the right end; older ones slide left.
        let first = s.frames_total as usize - s.frames.len();
        let lead = slots - s.frames.len();
        s.frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let v = ((f.level.clamp(FLOOR_MIN, 0.0) - FLOOR_MIN) / -FLOOR_MIN).powf(1.5);
                let h = FLOOR_REST_PX + (FLOOR_PX - FLOOR_REST_PX) * v;
                let x = origin.0 + (lead + i) as f32 * bar;
                let e = Element::shape(format!("floor-{}", first + i), ShapeKind::RRect)
                    .size((bar - 1.0).max(1.0), h)
                    .fill(l.accent.fade(0.35 + 0.4 * v))
                    .motion(Prop::Size, spring)
                    .motion(Prop::Position, spring)
                    .presence(Transition::fade(), Transition::fade());
                kit::at(e, x, base - h)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_guess_is_a_mind_change_and_new_or_dropped_words_are_not() {
        let w = |t: &str| place_words(t);
        let old = w("This is a longer test on");
        // "on" was a guess (place 5, after 5 committed words).
        assert_eq!(changed_places(&old, 5, &w("This is a longer test of voice")), vec![5]);
        assert!(changed_places(&old, 5, &w("This is a longer test on voice")).is_empty());
        assert!(changed_places(&old, 5, &w("This is a longer test")).is_empty());
        // A full stop that comes and goes changes no word.
        assert!(changed_places(&w("a test."), 1, &w("a test")).is_empty());
        // Committed places are not compared.
        assert!(changed_places(&old, 6, &w("That is a longer test on")).is_empty());
    }

    fn tok(text: &str, edge: u32) -> TailToken {
        TailToken {
            text: text.into(),
            start: 0.0,
            end: 0.08,
            edge,
            conf: 0.9,
            detail: None,
        }
    }

    #[test]
    fn tail_tokens_group_into_words_with_their_nearest_edge() {
        let tail = [
            tok(" of", 5),
            tok(" v", 4),
            tok("o", 3),
            tok("ice", 3),
            tok(" act", 1),
            tok("ing", 0),
        ];
        let words = tail_words(&tail, true);
        let got: Vec<(&str, u32, bool)> = words
            .iter()
            .map(|w| (w.text.as_str(), w.edge, w.joins))
            .collect();
        assert_eq!(
            got,
            [("of", 5, false), ("voice", 3, false), ("acting", 0, false)]
        );
    }

    #[test]
    fn a_first_token_without_a_space_continues_the_last_word() {
        let words = tail_words(&[tok("ivity", 4), tok(" det", 1)], true);
        assert!(words[0].joins);
        assert!(!words[1].joins);
        // With nothing committed it starts the text.
        assert!(!tail_words(&[tok("Th", 7)], false)[0].joins);
    }
}
