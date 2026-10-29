//! Typographic channels on the live text (`vo::scene::typo`, `word_voice`),
//! each behind a motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`):
//!
//! - `word-channels` (MO-03): every new word arrives with each channel
//!   moved (a left to right fill, a little heavier, looser and higher) and
//!   settles to plain type. Shows the channels themselves.
//! - `voice-ink` (MO-04): onset ink, pitch baseline and spoken-time
//!   tracking together, from what the voice did while each word was said.
//! - `spoken-stagger` (MO-05): words that come in one hypothesis settle in
//!   spoken order at a quarter of the spoken rhythm (200 to 600 ms in all).
//! - `onset-ink` (MO-13): a word born on a hard onset lands up to 0.6 px
//!   heavier and thins over 180 ms; a soft start arrives light and fills in.
//! - `pitch-baseline` (MO-14): the newest word's baseline follows the pitch
//!   by up to 3 px while it is said, and relaxes flat over 600 ms after.
//! - `spoken-tracking` (MO-15): a word arrives spaced by how long it took
//!   to say, a drawn-out word stretched, and settles.
//! - `confidence-ink` (MO-24): a tentative word is at 75 % ink and a little
//!   loose, with a short open seam under it. After two unchanged hypotheses
//!   it sets to full ink with a 1 px baseline settle and the seam closes.
//!
//! - `stroke-write` (SW-02): the pen drives each word's fill. The word's
//!   letters wait at 70 % ink and each goes to full as the pen writes it
//!   (`Popup::pen_fills`). Under reduced motion there is no pen and words
//!   are at full ink at once.
//!
//! The newest line stays inside the reading-line envelope: the framework
//! clamps baseline to 3 px, tracking to 5 % and ink to 70 % there. Under
//! reduced motion the channels jump to their rest values, so only the
//! state cues stay: tentative ink and an open seam.
//!
//! A look takes part by passing its live text through
//! [`Popup::typo_words`] and its text column through [`Popup::typo_marks`].

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use vo::scene::typo::{Typo, TypoMotion, Typos, WordTypo};
use vo::scene::word_voice::{VoiceTypo, WordVoices};
use vo_core::timeline::{spoken_offsets, SpokenFit};
use vo_core::words::WordId;

pub(crate) const CHANNELS: &str = "word-channels";
pub(crate) const VOICE: &str = "voice-ink";
pub(crate) const STAGGER: &str = "spoken-stagger";
pub(crate) const ONSET: &str = "onset-ink";
pub(crate) const PITCH: &str = "pitch-baseline";
pub(crate) const TRACKING: &str = "spoken-tracking";
pub(crate) const CONFIDENCE: &str = "confidence-ink";

const ALL: [&str; 7] = [CHANNELS, VOICE, STAGGER, ONSET, PITCH, TRACKING, CONFIDENCE];

/// Hypotheses a word must keep its text to count as stable (MO-24).
const STABLE_AFTER: u32 = 2;
/// The seam stays this long after its word stabilised, closing.
const SEAM_CLOSE_SECS: f64 = 0.5;
/// A cut scrolls the text up a line and settles within about this long (the
/// flow's drop hold and scroll spring); the stable settle waits until then.
/// 0.35 s still let a settle add 0.01 px to type's reading line at 424 px.
const SCROLL_SECS: f64 = 0.6;
/// The baseline relaxes flat over about this long after a word is said.
const PITCH_RELAX: f64 = 0.6;
/// Baseline per semitone of pitch (MO-14): twice the framework's default so the
/// rise reads at popup size; the reading line still clamps it at 3 px.
const PITCH_PX: f32 = 1.0;

/// What the channels keep between views.
#[derive(Default)]
pub(crate) struct State(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    voices: WordVoices,
    /// The last hypothesis seen, word by word.
    last: Vec<(WordId, String)>,
    /// Hypotheses each word has kept its text, and when it became stable.
    kept: HashMap<WordId, (u32, Option<f64>)>,
    /// Spoken-order delay of each word, fixed when the word first shows.
    delay: HashMap<WordId, f64>,
    /// The popup's cut (first word on screen) and when it last moved.
    cut: (usize, f64),
}

/// One word's channel state for this view.
#[derive(Clone, Copy)]
struct Seen {
    stable: bool,
    stable_at: Option<f64>,
    delay: f64,
}

impl Popup {
    fn channels_on(&self) -> bool {
        ALL.iter().any(|f| self.look.on(f)) || self.look.on(glyph_paths::WRITE)
    }

    fn channels_reduced(&self) -> bool {
        self.look.on("reduced-motion")
    }

    /// Brings the state up to the current hypothesis at `t`: hypothesis
    /// counts, spoken-order delays and voice readings.
    fn channels_sync(&self, t: f64) -> HashMap<WordId, Seen> {
        let mut s = self.channels.0.lock().unwrap();
        let words = self.model.words();
        let now: Vec<(WordId, String)> = words.iter().map(|w| (w.id, w.text.clone())).collect();
        if now != s.last {
            let mut kept = HashMap::new();
            for (id, text) in &now {
                let before = s.last.iter().find(|(i, _)| i == id).map(|(_, t)| t);
                let (n, at) = s.kept.get(id).copied().unwrap_or((0, None));
                let n = if before == Some(text) { n + 1 } else { 0 };
                let at = if n >= STABLE_AFTER { at.or(Some(t)) } else { None };
                kept.insert(*id, (n, at));
            }
            // New words settle in spoken order.
            let new: Vec<WordId> = now.iter().map(|w| w.0).filter(|id| !s.delay.contains_key(id)).collect();
            let times: Vec<Option<f64>> = new
                .iter()
                .map(|id| self.model.info(*id).and_then(|i| i.start).map(f64::from))
                .collect();
            let offsets = if new.len() > 1 {
                spoken_offsets(&times, SpokenFit::default())
            } else {
                vec![0.0; new.len()]
            };
            for (id, o) in new.iter().zip(offsets) {
                s.delay.insert(*id, o);
            }
            s.delay.retain(|id, _| now.iter().any(|w| w.0 == *id));
            s.kept = kept;
            s.last = now;
        }
        let bus = self.signals.get();
        s.voices.update(t, &self.model, &bus);
        words
            .iter()
            .map(|w| {
                // The engine's own count of kept hypotheses, when it sends one.
                let info = self.model.info(w.id);
                let (n, at) = s.kept.get(&w.id).copied().unwrap_or((0, None));
                let stable = info.map_or(n >= STABLE_AFTER, |i| i.settled >= STABLE_AFTER || n >= STABLE_AFTER);
                let stable_at = if stable { at.or(Some(t)) } else { None };
                let delay = s.delay.get(&w.id).copied().unwrap_or(0.0);
                (w.id, Seen { stable, stable_at, delay })
            })
            .collect()
    }

    /// The live text with the channels the look's features ask for.
    pub(crate) fn typo_words(&self, words: Element<Msg>) -> Element<Msg> {
        if !self.channels_on() {
            return words;
        }
        let l = &self.look;
        let t = self.extra.now;
        let seen = self.channels_sync(t);
        let mut s = self.channels.0.lock().unwrap();
        if s.cut.0 != self.cut {
            s.cut = (self.cut, t);
        }
        let scrolling = self.cut > 0 && t - s.cut.1 < SCROLL_SECS;
        let bus = self.signals.get();
        let vt = VoiceTypo {
            pitch_px: PITCH_PX,
            ..VoiceTypo::default()
        };
        let (onset, pitch, tracking) = (
            l.on(ONSET) || l.on(VOICE),
            // Under reduced motion the baseline stays flat.
            (l.on(PITCH) || l.on(VOICE)) && !(self.channels_reduced() || kit::reduced()),
            l.on(TRACKING) || l.on(VOICE),
        );
        let newest = self.shown.last().map(|w| w.id);
        let still = self.status().finishing || self.ending().is_some();
        let pen = self.pen_fills();
        let mut typos = Typos::new();
        for w in &self.shown {
            let Some(seen) = seen.get(&w.id).copied() else { continue };
            let letters = w.text.chars().count();
            let mut rest = Typo::REST;
            let mut arrive = Typo::REST;
            let mut motion = TypoMotion::default();
            if l.on(CHANNELS) {
                arrive = Typo { ink: 0.85, weight: 0.5, tracking: 1.05, baseline: 3.0, fill: 0.2 };
                motion.ink = Spring::critical_to_98(0.45);
                motion.fill = Spring::critical_to_98(0.35);
            }
            if let Some(v) = s.voices.get(w.id) {
                let a = vt.arrive(&v, letters);
                // Without any audio under the word there is no onset to show.
                if onset && v.peak > 0.0 {
                    (arrive.weight, arrive.fill) = (a.weight, a.fill);
                }
                if tracking {
                    arrive.tracking = a.tracking;
                    motion.tracking = Spring::critical_to_98(0.5);
                }
            }
            if pitch {
                motion.baseline = Spring::critical_to_98(PITCH_RELAX);
                if Some(w.id) == newest && !still {
                    // Follows the voice while the word is said.
                    rest.baseline = vt.baseline(s.voices.live_pitch(&bus, t));
                    motion.baseline = Spring::SNAP;
                }
            }
            let mut wt = WordTypo::rest(rest).motion(motion);
            if l.on(CONFIDENCE) {
                if seen.stable {
                    // Sets with a 1 px settle down, once, after a line scroll: on
                    // a scrolling line it would add to the scroll's own baseline
                    // travel on the reading line.
                    if !scrolling {
                        wt = wt.kick(1, Typo::REST.baseline(-1.0));
                    }
                } else if !still {
                    // Once the dictation is finishing, the final text lands
                    // at full ink: a revision arrives at 70 % of its rest, and
                    // 75 % of that would break the reading line's floor.
                    wt.rest.ink = 0.75;
                    wt.rest.tracking = 1.02;
                }
            }
            if arrive != Typo::REST {
                wt = wt.arrive(arrive);
            }
            if l.on(STAGGER) {
                // A small rise and weight, released in spoken order.
                if arrive == Typo::REST {
                    wt = wt.arrive(Typo::REST.baseline(2.0).weight(0.3).ink(0.8));
                }
                wt = wt.delay(seen.delay);
            }
            if let Some(keys) = pen.get(&w.id) {
                // Letters ahead of the pen keep the reading-line ink.
                wt = wt.floor(glyph_paths::PEN_FLOOR);
                typos.set_fill_track(w.id, keys.clone());
            }
            typos.set(w.id, wt);
        }
        words.word_typo(typos)
    }

    /// Marks under the words for the channels: the open seam under each
    /// tentative word (MO-24). `width` is the live text's wrap width; the
    /// marks are placed from the same layout the live text uses.
    pub(crate) fn typo_marks(&self, text: Element<Msg>, width: f32) -> Element<Msg> {
        self.typo_marks_for(text, &self.shown, width)
    }

    /// [`Popup::typo_marks`] for a live text that holds only `words`, such as
    /// one of karaoke's lines.
    pub(crate) fn typo_marks_for(&self, text: Element<Msg>, words: &[Word], width: f32) -> Element<Msg> {
        if !self.look.on(CONFIDENCE) || words.is_empty() {
            return text;
        }
        let l = &self.look;
        let t = self.extra.now;
        let seen = self.channels_sync(t);
        let reduced = self.channels_reduced();
        let style = l.text_style();
        let joined = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let laid = self.measure.layout(&joined, &style, Some(width));
        let mut starts = Vec::with_capacity(words.len());
        let mut at = 0usize;
        for w in words {
            starts.push(at);
            at += w.text.len() + 1;
        }
        let mut text = text;
        for (i, w) in words.iter().enumerate() {
            let Some(seen) = seen.get(&w.id) else { continue };
            let closing = seen.stable_at.map(|a| t - a);
            if closing.is_some_and(|c| c > SEAM_CLOSE_SECS) {
                continue;
            }
            let end = starts.get(i + 1).copied().unwrap_or(usize::MAX);
            let glyphs: Vec<_> = laid
                .glyphs
                .iter()
                .filter(|g| (g.cluster as usize) >= starts[i] && (g.cluster as usize) < end)
                .collect();
            let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else { continue };
            let (x0, x1) = (first.pos.x, last.pos.x + last.advance);
            let y = first.pos.y + 0.18 * style.size;
            // The seam: two hairlines with a gap in the middle that closes
            // when the word stabilises, then fades.
            let open = if seen.stable { 0.0 } else { (0.3 * (x1 - x0)).min(10.0) };
            let half = 0.5 * (x1 - x0 - open).max(0.0);
            let alpha = if seen.stable { 0.0 } else { 0.6 };
            let spring = if reduced { Spring::SNAP } else { Spring::critical_to_98(0.3) };
            for (k, x) in [(0, x0), (1, x1 - half)] {
                let seg = Element::shape(format!("seam-{}-{k}", w.id), ShapeKind::RRect)
                    .fill(l.accent.fade(alpha))
                    .motion(Prop::Size, spring)
                    .motion(Prop::Position, spring)
                    .motion(Prop::Opacity, spring)
                    .size(half, 1.0);
                text = text.child(kit::at(seg, x, y));
            }
        }
        text
    }
}
