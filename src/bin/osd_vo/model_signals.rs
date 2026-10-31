//! The live model's own signals on the live text (ASR-05), from
//! `preview-signals.json` (`voxtype::osd::live::ModelSignals`), each behind a
//! motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`). `ghost-tail` is in
//! every look's set (`defaults.rs`, LAT-1; `no-ghost-tail` turns it off);
//! the others are off unless asked for.
//!
//! - `ghost-tail`: the words the model hears in its lookahead (the newest
//!   0.56 s, decoded but not committed) follow the live text at 45 % ink,
//!   where the committed words will land. They show at once, with no fade
//!   or rise, and each step replaces them whole; when a word commits, the
//!   real word takes its place and the ghost goes in the same frame.
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
//!   fires only under `[osd] doubt_below` confidence, where it is on.
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
//! - `sound-strip`: the speech sounds the model has heard but not yet
//!   written as words, running ahead of the text: small phone labels
//!   (ARPAbet, lowercase) after the last word, one per run of 80 ms frames
//!   that the phone probe on encoder layer 8 hears as the same phone
//!   (ASR-10). Committed frames' sounds are firmer than the lookahead
//!   frames', which the next step hears again; silence shows nothing. As
//!   words commit, their sounds leave the strip. The worker sends the
//!   sounds only when `sound-strip` is in `[osd] motion` (it then runs the
//!   encoder with layer 8 as an extra output).
//!
//! Nothing shows once the recording stops, so the signals end with the
//! dictation and nothing moves at room tone. Under reduced motion the ghosts
//! swap at once (no fades) and the floor jumps between steps: the ink tiers
//! stay.
//!
//! A look takes part by passing its text column through
//! [`Popup::signal_marks`].

use super::*;

use vo::scene::text::Align;
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
pub(crate) const SOUND_STRIP: &str = "sound-strip";

/// Every model signal feature.
const ALL: [&str; 11] = [
    SOUND_STRIP,
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
/// Sound strip: the labels' size against the text, and the ink of a firm
/// and of a lookahead sound at probability 1.
const SOUND_SCALE: f32 = 0.55;
const SOUND_INK: f32 = 0.6;
const SOUND_TAIL_INK: f32 = 0.35;
/// Seconds per encoder frame.
const SOUND_FRAME_SECS: f32 = 0.08;

/// One run of frames the probe hears as the same sound.
#[derive(Debug, PartialEq)]
struct SoundRun {
    /// The first frame, counted from the start of the recording.
    frame: u32,
    frames: u32,
    /// The phone, lowercase ARPAbet.
    label: String,
    /// Its mean probability over the run.
    p: f32,
    /// From the lookahead frames.
    tail: bool,
}

/// The runs of the committed sounds, then of the lookahead sounds after
/// them, oldest first. Silence and frames with no sound leave gaps.
fn sound_runs(s: &ModelSignals) -> Vec<SoundRun> {
    let last = s.sounds.last().map_or(f32::NEG_INFINITY, |f| f.t);
    let frames = s.sounds.iter().map(|f| (f, false)).chain(
        s.sound_tail
            .iter()
            .filter(|f| f.t > last + 0.01)
            .map(|f| (f, true)),
    );
    let mut runs: Vec<SoundRun> = Vec::new();
    for (f, tail) in frames {
        let frame = (f.t / SOUND_FRAME_SECS).round() as u32;
        let Some(top) = f.top.first() else { continue };
        if top.text == "sil" {
            continue;
        }
        let label = top.text.to_ascii_lowercase();
        let p = top.logp.exp();
        match runs.last_mut() {
            Some(r) if r.label == label && r.tail == tail && r.frame + r.frames == frame => {
                r.p = (r.p * r.frames as f32 + p) / (r.frames + 1) as f32;
                r.frames += 1;
            }
            _ => runs.push(SoundRun {
                frame,
                frames: 1,
                label,
                p,
                tail,
            }),
        }
    }
    runs
}

/// The runner-up's text style, for text in `style`.
fn twin_style(style: &TextStyle) -> TextStyle {
    let mut small = *style;
    small.size *= TWIN_SCALE;
    small.line_height *= TWIN_SCALE;
    small
}

/// How far above a word's baseline the runner-up's box starts: its
/// baseline sits just above the word's ascenders.
fn twin_lift(style: &TextStyle) -> f32 {
    style.size * 0.95 + twin_style(style).line_height * 0.8
}

/// A token's text without its word marker, and whether it starts a word:
/// decoded tokens carry a leading space, raw SentencePiece ones a `▁`.
fn word_piece(token: &str) -> (bool, &str) {
    let piece = token.trim_start_matches([' ', '\u{2581}']);
    (piece.len() < token.len(), piece)
}

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
    /// After the last tail word drawn (x, line top), None when none is.
    drawn_end: Option<(f32, f32)>,
    /// The text's line height and how many lines it has room for.
    line_h: f32,
    /// With `breathing-stop`: the likeliest full stop in the tail, which is
    /// then left out of the tail words.
    stop: Option<f32>,
    /// The style the text is laid out in.
    style: TextStyle,
}

/// The live text a ghost tail follows: its style, the width it wraps at and
/// how many lines it has room for.
pub(crate) struct TailText {
    pub(crate) style: TextStyle,
    pub(crate) width: f32,
    pub(crate) rows: u32,
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

    /// How far the runner-up of `twin-spelling` reaches above the first
    /// line of text, for looks that clip the text at its top. 0 when the
    /// feature is off.
    pub(crate) fn twin_headroom(&self) -> f32 {
        if !self.look.on(TWIN_SPELLING) {
            return 0.0;
        }
        let style = self.look.text_style();
        let laid = self.measure.layout("Ag", &style, None);
        let Some(g) = laid.glyphs.first() else {
            return 0.0;
        };
        (twin_lift(&style) - g.pos.y).max(0.0).ceil() + 1.0
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

    /// The look's entry for a new word, or none while the model sends a
    /// ghost tail: a word that commits then takes the place its ghost held,
    /// at full ink, with no rise or fade.
    pub(crate) fn ghost_enter(&self, enter: Transition) -> Transition {
        let ghosts = self.look.on(GHOST_TAIL) || self.look.on(THREE_TIER);
        if ghosts && self.live_signals().is_some() {
            Transition::NONE
        } else {
            enter
        }
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
        let tail = TailText {
            style: self.look.text_style(),
            width,
            rows: self.lines,
        };
        self.signal_marks_in(text, words, &tail, origin, ink)
    }

    /// Where the drawn ghost words after `words` end (x, line top, in
    /// `tail`'s layout), when any are drawn. A look's caret or pen head
    /// that stands after the text goes here, so it never covers a ghost.
    pub(crate) fn ghost_tail_end(&self, words: &[Word], tail: &TailText) -> Option<(f32, f32)> {
        let l = &self.look;
        if !(l.on(GHOST_TAIL) || l.on(THREE_TIER)) {
            return None;
        }
        let s = self.live_signals()?;
        let laid = self.tail_layout(s, words, tail)?;
        laid.drawn_end
    }

    /// [`Popup::signal_marks`] for live text that is not the look's own text
    /// block: a single line of a look that sets each line apart, or text in
    /// another style. The ghost tail follows `tail`; the other signals keep
    /// the look's style.
    pub(crate) fn signal_marks_in(
        &self,
        text: Element<Msg>,
        words: &[Word],
        tail: &TailText,
        origin: (f32, f32),
        ink: Color,
    ) -> Element<Msg> {
        let width = tail.width;
        let forming = self.forming_on() && self.hearing.voice.active();
        if !forming {
            // A loading line's end is for this call's cue only.
            self.hearing.voice.hint.set(None);
        }
        if !self.signals_on() && !forming {
            return text;
        }
        let mut text = text;
        if forming {
            // After the ghost tail when there is one, else after the words.
            let none = ModelSignals {
                with_tail: self.files.preview.trim().to_string(),
                ..ModelSignals::default()
            };
            let s = self.live_signals().unwrap_or(&none);
            // A tail that is out of step with the words is left out.
            let (laid, tail_end) = match self.tail_layout(s, words, tail) {
                Some(l) => (Some(l), s.tail.last().map(|t| t.end as f64)),
                None => (self.tail_layout(&none, words, tail), None),
            };
            let end = laid.map_or((0.0, 0.0), |l| l.tail_end);
            if let Some(e) = self.forming_mark(words, end, tail_end, tail, origin, ink) {
                text = text.child(e);
            }
        }
        if !self.signals_on() {
            return text;
        }
        for e in self.graded_doubt(words, width, origin) {
            text = text.child(e);
        }
        for e in self.twin_spelling(words, width, origin, ink) {
            text = text.child(e);
        }
        if let Some(s) = self.live_signals() {
            let laid = self.tail_layout(s, words, tail);
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
            for e in self.sound_strip(s, laid.as_ref(), width, origin, ink) {
                text = text.child(e);
            }
            for e in self.commit_cursor(laid.as_ref(), origin) {
                text = text.child(e);
            }
            if let Some(e) = self.breathing_stop(laid.as_ref(), origin, ink) {
                text = text.child(e);
            }
            if let Some(e) = self.pre_echo_letters(s, laid.as_ref(), width, origin, ink) {
                text = text.child(e);
            }
        }
        text
    }

    /// Lays out `words` and the tail of `s` together. None when the tail
    /// does not belong to the text on screen: then one of the two files is
    /// newer than the other.
    fn tail_layout(&self, s: &ModelSignals, words: &[Word], text: &TailText) -> Option<TailLayout> {
        // On room tone the tail is empty: with only the ghost tail on there
        // is nothing to lay out.
        let ends = [COMMIT_CURSOR, BREATHING_STOP, PRE_ECHO_LETTERS, SOUND_STRIP];
        let forming = self.forming_on() && self.hearing.voice.active();
        if s.tail.is_empty() && !forming && !ends.iter().any(|f| self.look.on(f)) {
            return None;
        }
        let committed = self.files.preview.trim();
        if !s.with_tail.starts_with(committed) {
            return None;
        }
        let (style, width) = (text.style, text.width);
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
        // Laid out from the left edge; centred text is moved across below.
        let flat = TextStyle {
            align: Align::Start,
            ..style
        };
        let laid = self.measure.layout(&full, &flat, Some(width));
        let room = text.rows as f32 * line_h - 0.5;
        let top_of = |y: f32| (y / line_h).floor() * line_h;
        // The end of a run of text: after its last glyph.
        let end_of = |len: usize| -> (f32, f32) {
            match laid.glyphs.iter().rfind(|g| (g.cluster as usize) < len) {
                Some(g) => (g.pos.x + g.advance, top_of(g.pos.y)),
                None => (0.0, 0.0),
            }
        };
        let committed_end = end_of(committed_len);
        // How far `style.align` moves the line of glyph `i` across. The line
        // the committed text ends on sits where the live text centres it, so
        // its guesses follow the committed words; a line of guesses alone
        // centres on its own width.
        let k = style.align.factor();
        let shift = |i: usize| -> f32 {
            if k == 0.0 {
                return 0.0;
            }
            let top = top_of(laid.glyphs[i].pos.y);
            let w = if committed_len > 0 && (top - committed_end.1).abs() < 0.5 {
                committed_end.0
            } else {
                laid.lines
                    .iter()
                    .find(|l| l.glyphs.contains(&i))
                    .map_or(0.0, |l| l.width)
            };
            k * (width - w).max(0.0)
        };
        let mut drawn_end = None;
        let at = ranges
            .iter()
            .map(|range| {
                let i = laid
                    .glyphs
                    .iter()
                    .position(|g| range.contains(&(g.cluster as usize)))?;
                let g = &laid.glyphs[i];
                let top = top_of(g.pos.y);
                let x = g.pos.x + shift(i);
                let end = laid
                    .glyphs
                    .iter()
                    .filter(|g| range.contains(&(g.cluster as usize)))
                    .fold(x, |e, g| e.max(g.pos.x + shift(i) + g.advance));
                // A guess that would open a line the text has no room for
                // yet, or run past the edge of centred text, waits until the
                // text gets there.
                let fits = top < room && end <= width + 0.5;
                if fits {
                    drawn_end = Some((end, top));
                }
                fits.then_some((x, top))
            })
            .collect();
        let shifted = |(x, top): (f32, f32), len: usize| {
            let i = laid.glyphs.iter().rposition(|g| (g.cluster as usize) < len);
            // No text yet: a centred line starts in the middle.
            (x + i.map_or(k * width, shift), top)
        };
        // A tail that runs onto a line with no room yet ends, as far as the
        // screen goes, at the right edge of the last line there is.
        let mut tail_end = shifted(end_of(full.len()), full.len());
        if tail_end.1 >= room {
            tail_end = (width - CURSOR_GAP - CURSOR_PX, top_of(room - 1.0));
        }
        Some(TailLayout {
            tail,
            at,
            committed_end: shifted(committed_end, committed_len),
            tail_end,
            drawn_end,
            line_h,
            stop,
            style,
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
            let Some(d) = self.word_detail(i) else {
                continue;
            };
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
        let small = twin_style(&style);
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
            let Some(d) = self.word_detail(i) else {
                continue;
            };
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
            let y = first.pos.y - twin_lift(&style);
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
                let (fade, rise) = if reduced {
                    (0.7, 0.0)
                } else {
                    (1.0 - age, 4.0 * age)
                };
                let e = Element::label(format!("change-{p}-{old}"), old.clone())
                    .text_style(style)
                    .color(ink)
                    .opacity(CHANGE_INK * fade);
                out.push(kit::at(e, origin.0 + g.pos.x, origin.1 + top - rise));
            }
        }
        let n = st
            .flickers
            .iter()
            .filter(|&&f| now - f < FLICKER_SECS)
            .count();
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
                .size(
                    last.pos.x + last.advance - first.pos.x + 2.0 * grow,
                    h + 2.0 * grow,
                )
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
        let style = TextStyle {
            align: Align::Start,
            ..laid.style
        };
        // A ghost's key is the word's place in the dictation and its text: a
        // new guess at a place is a new element. Ghosts come and go at once,
        // so the newest words cost no entry time.
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
            // Its own width, so a narrow parent box cannot wrap it.
            let gw = self.measure.layout(&w.text, &style, None).size.w.ceil() + 1.0;
            let ghost = Element::label(format!("ghost-{place}-{}", w.text), w.text.as_str())
                .text_style(style)
                .size(gw, laid.line_h)
                .color(ink)
                .opacity(alpha)
                .motion(Prop::Opacity, Motion::Snap)
                .motion(Prop::Position, Motion::Snap)
                .presence(Transition::NONE, Transition::NONE);
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
            out.push(kit::at(
                e,
                origin.0 + x0 + CURSOR_GAP,
                origin.1 + t0 + inset + h,
            ));
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
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Option<Element<Msg>> {
        let l = &self.look;
        let laid = laid.filter(|_| l.on(PRE_ECHO_LETTERS))?;
        let ghosts = l.on(GHOST_TAIL) || l.on(THREE_TIER);
        // With the tail on screen, a frame the tail already wrote a token
        // for is no echo: its letters are there.
        let written = if ghosts {
            s.tail.last().map_or(f32::NEG_INFINITY, |t| t.start)
        } else {
            f32::NEG_INFINITY
        };
        let newest = s.frames.len().saturating_sub(STEP_FRAMES);
        let guess = s.frames[newest..]
            .iter()
            .rev()
            .filter(|f| f.level > ECHO_GATE && f.t > written + 0.01)
            .find_map(|f| f.top.first())?;
        let (starts_word, piece) = word_piece(&guess.text);
        let letters: String = piece.chars().take(ECHO_LETTERS).collect();
        if letters.is_empty() {
            return None;
        }
        let (mut x, top) = if ghosts {
            laid.tail_end
        } else {
            laid.committed_end
        };
        // A guess that starts a word sits one space after the text; only a
        // guess that continues the last word touches it.
        let style = l.text_style();
        if starts_word && x > 0.0 {
            let pair = self.measure.layout("aa", &style, None).size.w;
            x += self.measure.layout("a a", &style, None).size.w - pair;
        }
        let w = self.measure.layout(&letters, &style, None).size.w;
        if x + w > width {
            return None;
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

    /// `sound-strip`: the speech sounds the model has heard since the last
    /// word it wrote, after the text (after the ghost tail when it shows):
    /// one small phone label per run of frames, firm ones at
    /// [`SOUND_INK`] and lookahead ones at [`SOUND_TAIL_INK`], each times
    /// how sure the probe is. As many as fit on the line, oldest first.
    fn sound_strip(
        &self,
        s: &ModelSignals,
        laid: Option<&TailLayout>,
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        let Some(laid) = laid.filter(|_| l.on(SOUND_STRIP)) else {
            return Vec::new();
        };
        // The frames after the newest written word's last token.
        let written = self
            .files
            .live
            .as_ref()
            .and_then(|d| d.words.last())
            .and_then(|w| w.end)
            .unwrap_or(0.0);
        let from = (written / SOUND_FRAME_SECS).round() as u32;
        let ghosts = (l.on(GHOST_TAIL) || l.on(THREE_TIER)) && !s.tail.is_empty();
        let (mut x, top) = if ghosts {
            laid.tail_end
        } else {
            laid.committed_end
        };
        let style = laid.style;
        let mut small = style;
        small.size *= SOUND_SCALE;
        small.line_height *= SOUND_SCALE;
        let gap = self.measure.layout("a a", &small, None).size.w
            - self.measure.layout("aa", &small, None).size.w;
        if x > 0.0 {
            x += self.measure.layout("a a", &style, None).size.w
                - self.measure.layout("aa", &style, None).size.w;
        }
        // On the text's baseline: the small line's box sits as far below
        // the line top as its baseline is higher.
        let y = top + (laid.line_h - small.line_height) * 0.8;
        let presence = if self.signals_reduced() {
            (Transition::NONE, Transition::NONE)
        } else {
            (Transition::fade(), Transition::fade())
        };
        let mut out = Vec::new();
        for r in sound_runs(s).iter().filter(|r| r.frame + r.frames > from) {
            let w = self.measure.layout(&r.label, &small, None).size.w;
            if x + w > width {
                break;
            }
            let base = if r.tail { SOUND_TAIL_INK } else { SOUND_INK };
            let e = Element::label(format!("sound-{}", r.frame), r.label.clone())
                .text_style(small)
                .color(ink)
                .opacity(base * (0.35 + 0.65 * r.p))
                .presence(presence.0, presence.1);
            out.push(kit::at(e, origin.0 + x, origin.1 + y));
            x += w + gap;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(t: f32, phone: &str, p: f32) -> voxtype::osd::live::SoundFrame {
        voxtype::osd::live::SoundFrame {
            t,
            top: vec![voxtype::osd::live::Guess {
                text: phone.into(),
                logp: p.ln(),
            }],
        }
    }

    #[test]
    fn sounds_group_into_runs_with_gaps_for_silence() {
        let s = ModelSignals {
            sounds: vec![
                sound(0.0, "sil", 0.9),
                sound(0.08, "DH", 0.8),
                sound(0.16, "IH", 0.6),
                sound(0.24, "IH", 0.8),
                sound(0.32, "sil", 0.9),
                sound(0.40, "S", 0.5),
            ],
            // The lookahead starts after the committed frames; one that
            // overlaps them is left out.
            sound_tail: vec![
                sound(0.40, "Z", 0.9),
                sound(0.48, "S", 0.9),
                sound(0.56, "S", 0.7),
            ],
            ..ModelSignals::default()
        };
        let runs = sound_runs(&s);
        let got: Vec<_> = runs
            .iter()
            .map(|r| (r.frame, r.frames, r.label.as_str(), r.tail))
            .collect();
        assert_eq!(
            got,
            [
                (1, 1, "dh", false),
                (2, 2, "ih", false),
                (5, 1, "s", false),
                (6, 2, "s", true)
            ]
        );
        assert!((runs[1].p - 0.7).abs() < 1e-5);
        assert!((runs[3].p - 0.8).abs() < 1e-5);
        assert!(sound_runs(&ModelSignals::default()).is_empty());
    }

    #[test]
    fn a_token_with_a_word_marker_starts_a_word() {
        assert_eq!(word_piece(" t"), (true, "t"));
        assert_eq!(word_piece("\u{2581}t"), (true, "t"));
        assert_eq!(word_piece("ion"), (false, "ion"));
    }

    #[test]
    fn a_changed_guess_is_a_mind_change_and_new_or_dropped_words_are_not() {
        let w = |t: &str| place_words(t);
        let old = w("This is a longer test on");
        // "on" was a guess (place 5, after 5 committed words).
        assert_eq!(
            changed_places(&old, 5, &w("This is a longer test of voice")),
            vec![5]
        );
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

    /// s3th: a caret or pen head that stands after the text goes after the
    /// drawn ghost words, not over the first one.
    #[test]
    fn the_tail_end_is_after_the_last_drawn_ghost() {
        let fonts = super::super::swiss_look::edit_check::fonts();
        let cfg = OsdConfig {
            theme: OsdTheme::Scope,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &PlasmaTheme::default(), String::new(), &fonts);
        look.set_motion(&cfg.motion);
        assert!(look.on(GHOST_TAIL));
        let mut popup = Popup::new(look.clone(), &fonts);
        let committed = "This is a longer test";
        popup.files.recording = true;
        popup.files.preview = committed.into();
        popup.model_signals = Some(ModelSignals {
            tail: vec![tok(" of", 5), tok(" voice", 3)],
            with_tail: format!("{committed} of voice"),
            ..ModelSignals::default()
        });
        let words: Vec<Word> = committed
            .split(' ')
            .enumerate()
            .map(|(i, w)| Word {
                id: i as u32 + 1,
                text: w.into(),
            })
            .collect();
        let style = look.text_style();
        let text = TailText {
            style,
            width: 2000.0,
            rows: 1,
        };
        let width = |t: &str| popup.measure.layout(t, &style, None).size.w;
        let (x, top) = popup.ghost_tail_end(&words, &text).expect("ghosts drawn");
        assert_eq!(top, 0.0);
        let want = width(&format!("{committed} of voice"));
        assert!((x - want).abs() < 2.0, "{x} for {want}");
        assert!(x > width(&format!("{committed} of")) + 1.0);
        // With no tail there is nothing to stand after.
        popup.model_signals = None;
        assert!(popup.ghost_tail_end(&words, &text).is_none());
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
