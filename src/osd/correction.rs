//! Spoken corrections in the live and the final text.
//!
//! While the correction key is held (`HotkeyEvent::CorrectionStart`), the
//! live words spoken in the hold leave the text and become the correction.
//! Once the live model has caught up with the end of the hold,
//! [`crate::text::align::place`] finds the words they replace among the
//! words spoken before it. A placed correction is kept by audio time, not
//! by word index, so it survives the live model revising its words. At the
//! stop, [`apply_final`] makes the same replacements in the final text.
//!
//! The daemon owns one [`Corrections`] per dictation in a static, like
//! [`crate::osd::live`]: `live::preview` runs each new word list through
//! [`overlay`], and `preview.json` carries a [`CorrectionView`] for the popup.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::osd::live::{TimedWord, WordDetail};
use crate::text::align::{self, Refused};
use crate::text::{learn, spell};

/// How much audio past the key coming up the live model hears before a
/// correction stops waiting for the words of the hold, when no word after
/// the hold has arrived sooner. In seconds of the recording's audio clock,
/// so a keyboard edit's pause, which stops that clock, does not count.
pub const SETTLE: f32 = 1.2;

/// Words of slack when a correction is placed in the final text, which has
/// no timing: its search ends this many words after the live word count.
const FINAL_SLACK_WORDS: usize = 3;

/// What the popup shows of the corrections, in `preview.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CorrectionView {
    /// The words heard so far while the key is held; None when no hold is
    /// open, empty when one is open but no word arrived yet.
    pub hearing: Option<String>,
    /// A hold ended and waits for the live model's words.
    #[serde(default)]
    pub pending: bool,
    /// The letters spelled so far, when the open hold starts with "spell"
    /// ([`crate::text::spell`]).
    #[serde(default)]
    pub spelling: Option<String>,
    /// The newest outcome.
    pub note: Option<CorrectionNote>,
}

/// The outcome of one correction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrectionNote {
    /// Counts outcomes in this dictation, so a popup can tell a new one.
    pub id: u32,
    /// The words replaced; empty when nothing changed.
    pub from: String,
    /// The words heard in the hold.
    pub to: String,
    /// Why nothing changed; None when the correction was placed.
    pub dropped: Option<String>,
    /// The correction will be learned ([`crate::text::learn`]) once pasted.
    #[serde(default)]
    pub learned: bool,
}

/// A live word and its detail.
pub type Word = (TimedWord, Option<WordDetail>);

#[derive(Debug, Clone, PartialEq)]
enum State {
    Open,
    /// The key came up; the hold waits for its words.
    Pending,
    /// Words timed `from..=to` (`wrong`) are replaced by `right`.
    Placed {
        from: f32,
        to: f32,
        wrong: String,
        right: Vec<String>,
        /// `right` is one word spelled in the hold, not the words heard.
        spelled: bool,
    },
    Dropped,
}

/// A hold as the live worker reads it from [`windows_path`]: seconds into
/// the recording, `t1` None while the key is still down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSpec {
    pub t0: f32,
    pub t1: Option<f32>,
}

/// What the final engine made of one hold, in the worker's response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FinalClip {
    pub t0: f32,
    pub t1: f32,
    /// The hold's audio alone, decoded with the replaced words pushed down.
    pub text: String,
    /// The hold's audio was cut out of every part of the final text, so the
    /// final text has no copy of the respoken words.
    pub cut: bool,
}

/// `correction-windows.json` in the runtime folder: the holds of the
/// current dictation, for the live worker.
pub fn windows_path() -> std::path::PathBuf {
    crate::config::Config::runtime_dir().join("correction-windows.json")
}

/// The holds in [`windows_path`]; none when there is no file.
pub fn read_windows() -> Vec<WindowSpec> {
    std::fs::read_to_string(windows_path())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
struct Window {
    /// Seconds into the recording, from key-down to key-up.
    t0: f32,
    t1: f32,
    state: State,
}

/// The corrections of one dictation.
#[derive(Debug, Default)]
pub struct Corrections {
    windows: Vec<Window>,
    /// The live model's newest word list, before any correction.
    raw: Vec<Word>,
    notes: u32,
    note: Option<CorrectionNote>,
    /// The final engine's text of each hold, when the worker gave them.
    clips: Vec<FinalClip>,
    /// The corrections [`Corrections::apply_final`] made, for the log.
    applied: Vec<learn::Entry>,
    /// Words whose middle is before this many seconds are frozen by a
    /// keyboard edit: no correction placed from now on replaces them.
    frozen_until: Option<f32>,
}

fn middle(w: &TimedWord) -> f32 {
    (w.start + w.end) / 2.0
}

fn inside(w: &TimedWord, t0: f32, t1: f32) -> bool {
    (t0..t1).contains(&middle(w))
}

impl Corrections {
    /// The key went down `t0` seconds into the recording and was held past
    /// its threshold. Ignored while another hold is open.
    pub fn start(&mut self, t0: f32) {
        if self.open().is_none() {
            self.windows.push(Window {
                t0,
                t1: f32::INFINITY,
                state: State::Open,
            });
        }
    }

    /// The key came up `t1` seconds into the recording.
    pub fn end(&mut self, t1: f32) {
        if let Some(w) = self.open() {
            w.t1 = t1.max(w.t0);
            w.state = State::Pending;
        }
    }

    /// Another key was pressed during the hold: its words stay dictation.
    pub fn cancel(&mut self) {
        self.windows.retain(|w| w.state != State::Open);
    }

    /// A keyboard edit froze the text spoken before `t` seconds. Holds
    /// that ended are placed first, so a correction made just before the
    /// edit tap still counts; from then on no correction replaces a word
    /// spoken before `t`.
    pub fn freeze(&mut self, t: f32) {
        self.render_with(None, true);
        self.frozen_until = Some(self.frozen_until.map_or(t, |f| f.max(t)));
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty() && self.note.is_none()
    }

    fn open(&mut self) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.state == State::Open)
    }

    /// The holds, for the live worker.
    pub fn specs(&self) -> Vec<WindowSpec> {
        self.windows
            .iter()
            .map(|w| WindowSpec {
                t0: w.t0,
                t1: (w.state != State::Open).then_some(w.t1),
            })
            .collect()
    }

    /// A new live word list: the words to show, with the corrections made.
    /// `heard`: seconds of audio the live model has decoded. Holds it has
    /// heard `SETTLE` past, or heard words after, are placed now.
    pub fn overlay(&mut self, raw: Vec<Word>, heard: f32) -> Vec<Word> {
        self.raw = raw;
        self.render(Some(heard))
    }

    /// The words to show for the current list, placing nothing new.
    pub fn current(&mut self) -> Vec<Word> {
        self.render(None)
    }

    /// `heard`: place pending holds that are ready with this much audio
    /// decoded; None places none, and `force` (in [`Self::apply_final`])
    /// places all.
    fn render(&mut self, heard: Option<f32>) -> Vec<Word> {
        self.render_with(heard, false)
    }

    fn render_with(&mut self, heard: Option<f32>, force: bool) -> Vec<Word> {
        let mut out = self.raw.clone();
        for i in 0..self.windows.len() {
            let Window { t0, t1, .. } = self.windows[i];
            out.retain(|(w, _)| !inside(w, t0, t1));
            if self.windows[i].state == State::Pending {
                let heard_past = self.raw.iter().any(|(w, _)| w.start >= t1);
                let due = heard.is_some_and(|h| h >= t1 + SETTLE);
                if force || (heard.is_some() && (heard_past || due)) {
                    self.windows[i].state = self.place(&out, t0, t1);
                }
            }
            if let State::Placed {
                from, to, right, ..
            } = &self.windows[i].state
            {
                replace_by_time(&mut out, *from, *to, right);
            }
        }
        out
    }

    /// Places the words heard in `t0..t1` among the words of `shown` that
    /// were spoken before `t0`, and notes the outcome.
    fn place(&mut self, shown: &[Word], t0: f32, t1: f32) -> State {
        let heard: Vec<String> = self
            .raw
            .iter()
            .filter(|(w, _)| inside(w, t0, t1))
            .map(|(w, _)| w.text.clone())
            .collect();
        let before: Vec<&TimedWord> = shown
            .iter()
            .map(|(w, _)| w)
            .filter(|w| middle(w) < t0)
            .collect();
        let texts: Vec<&str> = before.iter().map(|w| w.text.as_str()).collect();
        // The first word a correction may replace.
        let open = self
            .frozen_until
            .map_or(0, |f| before.iter().take_while(|w| middle(w) < f).count());
        self.notes += 1;
        let mut note = CorrectionNote {
            id: self.notes,
            from: String::new(),
            to: heard.join(" "),
            dropped: None,
            learned: false,
        };
        let spelling = spell::is_spelling(&heard);
        let target: Vec<String> = if spelling {
            spell::spelled(&heard).into_iter().collect()
        } else {
            heard.clone()
        };
        let placed = if heard.is_empty() {
            Err("nothing heard")
        } else if target.is_empty() {
            Err("not letters")
        } else {
            align::place(&texts[open..], &target)
                .map(|span| span.start + open..span.end + open)
                .map_err(|r| {
                    if open > 0 && align::place(&texts, &target).is_ok() {
                        return "frozen text";
                    }
                    match r {
                        Refused::Empty => "nothing to replace",
                        Refused::NoMatch { .. } => "no match",
                        Refused::Ambiguous { .. } => "two places fit",
                        Refused::TooShort { .. } => "too short to place",
                    }
                })
        };
        let state = match placed {
            Ok(span) => {
                note.from = texts[span.start..span.end].join(" ");
                let right = if spelling {
                    note.to = target.join(" ");
                    note.to.clone()
                } else {
                    clip_words(&note.to, &note.from).join(" ")
                };
                note.learned =
                    learn::active().is_some_and(|l| l.would_learn(&note.from, &right, spelling));
                State::Placed {
                    from: before[span.start].start,
                    to: before[span.end - 1].end,
                    wrong: note.from.clone(),
                    right: target,
                    spelled: spelling,
                }
            }
            Err(reason) => {
                note.dropped = Some(reason.to_string());
                State::Dropped
            }
        };
        tracing::info!(
            from = %note.from,
            to = %note.to,
            dropped = ?note.dropped,
            "Spoken correction"
        );
        self.note = Some(note);
        state
    }

    /// What the popup shows; None when there is nothing to show.
    pub fn view(&self) -> Option<CorrectionView> {
        if self.is_empty() {
            return None;
        }
        let heard: Option<Vec<&str>> =
            self.windows
                .iter()
                .find(|w| w.state == State::Open)
                .map(|w| {
                    self.raw
                        .iter()
                        .filter(|(word, _)| inside(word, w.t0, w.t1))
                        .map(|(word, _)| word.text.as_str())
                        .collect()
                });
        Some(CorrectionView {
            hearing: heard.as_ref().map(|h| h.join(" ")),
            spelling: heard
                .as_deref()
                .and_then(spell::letters)
                .map(|(letters, _)| letters),
            pending: self.windows.iter().any(|w| w.state == State::Pending),
            note: self.note.clone(),
        })
    }

    /// The final text with the corrections made. Each correction replaces
    /// the words it was placed on (the newest match before the hold's
    /// start). Its words are the final engine's own text of the hold when
    /// the worker gave one ([`FinalClip`]) that differs from the replaced
    /// words and places in the final text, else the live words. When the hold's audio was not cut out of
    /// the final text, the engine heard the respoken words as well, so that
    /// copy (the newest match before the hold's end) goes first. The final
    /// text has no timing: the live word counts say how far to look. A
    /// text with no placed correction comes back unchanged; when the newest
    /// correction places nowhere, its note says so.
    pub fn apply_final(&mut self, text: &str) -> String {
        self.render_with(None, true);
        self.applied.clear();
        // Each placed hold with the words to try: the clip's, then the live
        // words for when the clip's words fit nowhere in the final text. A
        // spelled word is tried alone, as spelled.
        let placed: Vec<FinalHold> = self
            .windows
            .iter()
            .filter_map(|w| match &w.state {
                State::Placed {
                    wrong,
                    right,
                    spelled: true,
                    ..
                } => {
                    let cut = self
                        .clips
                        .iter()
                        .any(|c| (c.t0 - w.t0).abs() < 0.05 && c.cut);
                    (right.join(" ") != *wrong)
                        .then(|| (w.t0, w.t1, vec![right.clone()], cut, true))
                }
                State::Placed { wrong, right, .. } => {
                    let clip = self.clips.iter().find(|c| (c.t0 - w.t0).abs() < 0.05);
                    let cut = clip.is_some_and(|c| c.cut);
                    let mut tries: Vec<Vec<String>> = clip
                        .map(|c| clip_words(&c.text, wrong))
                        .into_iter()
                        .chain([clip_words(&right.join(" "), wrong)])
                        .filter(|words| !words.is_empty() && !same_words(&words.join(" "), wrong))
                        .collect();
                    tries.dedup();
                    if tries.is_empty() {
                        tracing::info!(%wrong, "Correction heard as the same words: spell it");
                        return None;
                    }
                    Some((w.t0, w.t1, tries, cut, false))
                }
                _ => None,
            })
            .collect();
        if placed.is_empty() {
            return text.to_string();
        }
        let mut words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
        // How many more words the final text has than the live count says,
        // from the corrections already made.
        let mut shift: isize = 0;
        let before = |t: f32, shift: isize| {
            let n = self.raw.iter().filter(|(w, _)| middle(w) < t).count() as isize;
            (n + shift).max(0) as usize + FINAL_SLACK_WORDS
        };
        let newest = placed.last().map(|p| p.0);
        let mut newest_missed = false;
        for (t0, t1, tries, cut, spelled) in placed {
            let said: Vec<String> = self
                .raw
                .iter()
                .filter(|(w, _)| inside(w, t0, t1))
                .map(|(w, _)| w.text.clone())
                .collect();
            // The copy of the hold in the final text: the letters as said
            // when spelled, else the words tried.
            let copies = if spelled {
                std::slice::from_ref(&said)
            } else {
                &tries[..]
            };
            // The words before the hold, by the count before any of its
            // respoken words go.
            let mut hold_start = before(t0, shift);
            if cut {
                // The live count includes the respoken words; the text not.
                shift -= said.len() as isize;
            } else if let Some(copy) = copies
                .iter()
                .find_map(|right| align::place_before(&words, before(t1, shift), right).ok())
            {
                words.drain(copy.start..copy.end);
                shift -= (copy.end - copy.start) as isize;
                hold_start = hold_start.min(copy.start);
            }
            let found = tries.into_iter().find_map(|right| {
                align::place_before(&words, hold_start, &right)
                    .ok()
                    .map(|span| (span, right))
            });
            if let Some((span, right)) = found {
                shift += right.len() as isize - (span.end - span.start) as isize;
                self.applied.push(learn::Entry {
                    time: learn::now(),
                    wrong: words[span.start..span.end].join(" "),
                    right: right.join(" "),
                    live: self
                        .raw
                        .iter()
                        .map(|(w, _)| w.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    final_text: text.to_string(),
                    t0,
                    t1,
                    spelled,
                    ..learn::Entry::default()
                });
                words.splice(span.start..span.end, right);
            } else {
                tracing::warn!(t0, "Correction not found in the final text");
                newest_missed |= newest == Some(t0);
            }
        }
        if newest_missed {
            // A new outcome, so a popup that showed "Corrected" shows it.
            self.notes += 1;
            if let Some(note) = &mut self.note {
                note.id = self.notes;
                note.from.clear();
                note.dropped = Some("not in the final text".into());
                note.learned = false;
            }
        }
        let pasted = words.join(" ");
        for entry in &mut self.applied {
            entry.pasted = pasted.clone();
        }
        pasted
    }
}

/// A placed hold in [`Corrections::apply_final`]: its start and end, the
/// words to try in turn, whether its audio was cut out of the final text,
/// and whether the words were spelled.
type FinalHold = (f32, f32, Vec<Vec<String>>, bool, bool);

/// Whether `a` and `b` are the same words, ignoring case and punctuation.
/// "FrankenTerm" and "franken term" are not: joining words is a fix.
fn same_words(a: &str, b: &str) -> bool {
    let key = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(word_key)
            .filter(|w| !w.is_empty())
            .collect()
    };
    key(a) == key(b)
}

/// A word in lower case without its punctuation.
fn word_key(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The words of a hold's final text, fitted into the sentence: the final
/// engine writes a clip alone as a sentence ("Popup."), so the full stop
/// goes, and the first word's capital too when the words it replaces were
/// lower case and the capital is only the sentence's (not "FrankenTerm").
fn clip_words(text: &str, wrong: &str) -> Vec<String> {
    let text = text.trim().trim_end_matches(['.', '!', '?']);
    let mut words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    let wrong_lower = wrong.chars().next().is_some_and(char::is_lowercase);
    if let Some(first) = words.first_mut() {
        let mut chars = first.chars();
        let sentence_capital =
            chars.next().is_some_and(char::is_uppercase) && chars.all(|c| !c.is_uppercase());
        if wrong_lower && sentence_capital {
            *first = first.to_lowercase();
        }
    }
    words
}

/// The words each hold replaces in `raw`, the live words with no correction
/// made: what the final pass pushes down when it decodes the hold. A word
/// that is also among the respoken words stays out, so the push does not
/// spoil the right words ("test" when "longer test" is respoken as "quiet
/// test"). None for a hold that places nowhere or replaces only such words.
pub fn wrong_spans(raw: Vec<Word>, holds: &[(f32, f32)]) -> Vec<Option<String>> {
    let mut c = Corrections::default();
    for &(t0, t1) in holds {
        c.start(t0);
        c.end(t1);
    }
    c.raw = raw;
    c.render_with(None, true);
    c.windows
        .iter()
        .map(|w| match &w.state {
            State::Placed { wrong, right, .. } => {
                let right: Vec<String> = right.iter().map(|w| word_key(w)).collect();
                let kept: Vec<&str> = wrong
                    .split_whitespace()
                    .filter(|w| !right.contains(&word_key(w)))
                    .collect();
                (!kept.is_empty()).then(|| kept.join(" "))
            }
            _ => None,
        })
        .collect()
}

/// Replaces the words whose middle falls in `from..=to` with `right`,
/// spread over that time, where the first of them stood (or where they
/// would stand, if the live model has since moved them).
fn replace_by_time(words: &mut Vec<Word>, from: f32, to: f32, right: &[String]) {
    let at = words
        .iter()
        .position(|(w, _)| middle(w) >= from)
        .unwrap_or(words.len());
    words.retain(|(w, _)| !(from..=to).contains(&middle(w)));
    let step = (to - from) / right.len().max(1) as f32;
    let new = right.iter().enumerate().map(|(i, text)| {
        let start = from + step * i as f32;
        let word = TimedWord {
            text: text.clone(),
            start,
            end: start + step,
            conf: None,
        };
        (word, None)
    });
    let at = at.min(words.len());
    words.splice(at..at, new);
}

static CORRECTIONS: Mutex<Option<Corrections>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Corrections) -> T) -> T {
    let mut guard = CORRECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Corrections::default))
}

/// Writes the holds to [`windows_path`] for the live worker, which cuts them
/// out of the final text's audio.
fn write_windows(c: &Corrections) {
    let path = windows_path();
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string(&c.specs()) {
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// A hold began `t0` seconds into the recording.
pub fn start(t0: f32) {
    with(|c| {
        c.start(t0);
        write_windows(c);
    });
}

/// The hold ended `t1` seconds into the recording.
pub fn end(t1: f32) {
    with(|c| {
        c.end(t1);
        write_windows(c);
    });
}

/// The hold was dropped.
pub fn cancel() {
    with(|c| {
        c.cancel();
        write_windows(c);
    });
}

/// The final engine's text of each hold, from the worker's response.
pub fn final_clips(clips: Vec<FinalClip>) {
    with(|c| c.clips = clips);
}

/// A keyboard edit froze the text spoken before `t` seconds.
pub fn freeze(t: f32) {
    with(|c| c.freeze(t));
}

/// The dictation ended.
pub fn clear() {
    *CORRECTIONS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let _ = std::fs::remove_file(windows_path());
}

/// A new live word list, with the corrections made. `heard`: seconds of
/// audio the live model has decoded.
pub fn overlay(raw: Vec<Word>, heard: f32) -> Vec<Word> {
    with(|c| c.overlay(raw, heard))
}

/// The current word list with the corrections made, placing nothing new.
pub fn current() -> Vec<Word> {
    with(Corrections::current)
}

/// Whether any correction was started in this dictation.
pub fn active() -> bool {
    with(|c| !c.is_empty())
}

pub fn view() -> Option<CorrectionView> {
    with(|c| c.view())
}

/// The final text with this dictation's corrections made.
pub fn apply_final(text: &str) -> String {
    with(|c| c.apply_final(text))
}

/// The corrections the last [`apply_final`] made, for the corrections log.
pub fn applied() -> Vec<learn::Entry> {
    with(|c| c.applied.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words a quarter second apart from `t`, as the live model times them.
    fn spoken(t: f32, text: &str) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| {
                let start = t + 0.25 * i as f32;
                let word = TimedWord {
                    text: w.into(),
                    start,
                    end: start + 0.2,
                    conf: Some(0.9),
                };
                (word, None)
            })
            .collect()
    }

    fn text(words: &[Word]) -> String {
        words
            .iter()
            .map(|(w, _)| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Live word lists as the model sends them in steps: everything said so
    /// far, plus `later` once it has caught up.
    fn steps(parts: &[(f32, &str)]) -> Vec<Vec<Word>> {
        (1..=parts.len())
            .map(|n| parts[..n].iter().flat_map(|(t, s)| spoken(*t, s)).collect())
            .collect()
    }

    #[test]
    fn a_respoken_word_replaces_the_words_it_sounds_like() {
        // "use franken term here" at 0 s, the hold from 1.2 s to 2.0 s with
        // "FrankenTerm" at 1.3 s, then "and run it" at 2.4 s.
        let lists = steps(&[
            (0.0, "use franken term here"),
            (1.3, "FrankenTerm"),
            (2.4, "and run it"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        assert_eq!(
            text(&c.overlay(lists[0].clone(), t)),
            "use franken term here"
        );

        c.start(1.2);
        let shown = c.overlay(lists[1].clone(), t);
        assert_eq!(text(&shown), "use franken term here");
        assert_eq!(c.view().unwrap().hearing.as_deref(), Some("FrankenTerm"));

        c.end(2.0);
        assert!(c.view().unwrap().pending);
        // The next words after the hold settle it.
        let shown = c.overlay(lists[2].clone(), t);
        assert_eq!(text(&shown), "use FrankenTerm here and run it");
        let view = c.view().unwrap();
        assert_eq!(view.hearing, None);
        assert!(!view.pending);
        let note = view.note.unwrap();
        assert_eq!(
            (note.from.as_str(), note.to.as_str()),
            ("franken term", "FrankenTerm")
        );
        assert_eq!(note.dropped, None);
    }

    #[test]
    fn a_placed_correction_survives_a_revision_of_its_words() {
        let lists = steps(&[
            (0.0, "use franken term here"),
            (1.3, "FrankenTerm"),
            (2.4, "and"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0);
        c.overlay(lists[2].clone(), t);
        // The live model revises "franken term" to "frank and term".
        let mut revised = lists[2].clone();
        revised[1].0.text = "frank".into();
        revised[2].0.text = "and-term".into();
        assert_eq!(text(&c.overlay(revised, t)), "use FrankenTerm here and");
    }

    /// R20: the wait is audio the live model decoded past the key coming
    /// up, so a keyboard edit's pause, which stops that clock, does not
    /// place a hold early.
    #[test]
    fn a_hold_settles_after_the_wait_with_no_words_after_it() {
        let lists = steps(&[(0.0, "use franken term here"), (1.3, "FrankenTerm")]);
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0);
        for heard in [1.6, 2.0, 2.0 + SETTLE - 0.05] {
            assert_eq!(
                text(&c.overlay(lists[1].clone(), heard)),
                "use franken term here",
                "{heard} s decoded"
            );
            assert!(c.view().unwrap().pending);
        }
        let later = 2.0 + SETTLE;
        assert_eq!(
            text(&c.overlay(lists[1].clone(), later)),
            "use FrankenTerm here"
        );
    }

    #[test]
    fn a_correction_that_fits_nowhere_is_dropped_with_a_reason() {
        let lists = steps(&[
            (0.0, "the cat sat on the mat"),
            (1.8, "Kubernetes"),
            (2.6, "again"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.7);
        c.end(2.4);
        assert_eq!(
            text(&c.overlay(lists[2].clone(), t)),
            "the cat sat on the mat again"
        );
        let note = c.view().unwrap().note.unwrap();
        assert_eq!(note.dropped.as_deref(), Some("no match"));
        assert_eq!(note.from, "");
    }

    #[test]
    fn a_cancelled_hold_gives_its_words_back() {
        let lists = steps(&[(0.0, "use franken term here"), (1.3, "FrankenTerm")]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.2);
        assert_eq!(
            text(&c.overlay(lists[1].clone(), t)),
            "use franken term here"
        );
        c.cancel();
        assert_eq!(text(&c.current()), "use franken term here FrankenTerm");
        assert!(c.is_empty());
        assert_eq!(c.view(), None);
    }

    #[test]
    fn the_final_text_gets_the_correction_and_loses_the_respoken_copy() {
        let lists = steps(&[
            (0.0, "use franken term here"),
            (1.3, "FrankenTerm"),
            (2.4, "and run it"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0);
        c.overlay(lists[2].clone(), t);
        assert_eq!(
            c.apply_final("Use Franken Term here. FrankenTerm And run it."),
            "Use FrankenTerm here. And run it."
        );
    }

    #[test]
    fn a_stop_right_after_the_hold_still_corrects_the_final_text() {
        // The key came up and the dictation stopped before the live model
        // caught up: the final text places the correction on its own.
        let lists = steps(&[(0.0, "use franken term here"), (1.3, "FrankenTerm")]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.2);
        c.overlay(lists[1].clone(), t);
        c.end(2.0);
        assert_eq!(
            c.apply_final("use franken term here FrankenTerm"),
            "use FrankenTerm here"
        );
    }

    fn placed_hold() -> Corrections {
        let lists = steps(&[
            (0.0, "use franken term here"),
            (1.3, "FrankenTerm"),
            (2.4, "and run it"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0);
        c.overlay(lists[2].clone(), t);
        c
    }

    #[test]
    fn the_final_engines_own_clip_replaces_the_words_in_a_cut_final_text() {
        let mut c = placed_hold();
        c.clips = vec![FinalClip {
            t0: 1.2,
            t1: 2.0,
            text: "Franken-Term.".into(),
            cut: true,
        }];
        assert_eq!(
            c.apply_final("Use franken term here and run it."),
            "Use Franken-Term here and run it."
        );
    }

    #[test]
    fn a_clip_heard_as_the_same_words_falls_back_to_the_live_words() {
        let mut c = placed_hold();
        c.clips = vec![FinalClip {
            t0: 1.2,
            t1: 2.0,
            text: "Franken term.".into(),
            cut: true,
        }];
        assert_eq!(
            c.apply_final("Use franken term here and run it."),
            "Use FrankenTerm here and run it."
        );
    }

    #[test]
    fn clip_words_drop_the_sentence_marks_of_a_clip_alone() {
        assert_eq!(clip_words("Popup.", "pop up"), ["popup"]);
        assert_eq!(clip_words("FrankenTerm.", "franken term"), ["FrankenTerm"]);
        assert_eq!(clip_words("Oslo!", "Oslo"), ["Oslo"]);
        assert_eq!(clip_words("Paris", "parish"), ["paris"]);
    }

    #[test]
    fn wrong_spans_name_the_words_each_hold_replaces() {
        let mut raw = spoken(0.0, "use franken term here");
        raw.extend(spoken(1.3, "FrankenTerm"));
        raw.extend(spoken(2.4, "the cat sat"));
        raw.extend(spoken(3.4, "Kubernetes"));
        let spans = wrong_spans(raw, &[(1.2, 2.0), (3.3, 4.0)]);
        assert_eq!(spans, [Some("franken term".to_string()), None]);
    }

    #[test]
    fn wrong_spans_leave_out_the_respoken_words() {
        let mut raw = spoken(0.0, "this is a longer test of voice");
        raw.extend(spoken(2.4, "quiet test"));
        let spans = wrong_spans(raw, &[(2.3, 3.0)]);
        assert_eq!(spans.len(), 1);
        let span = spans[0].clone().unwrap_or_default();
        assert!(!span.contains("test"), "{span:?}");
    }

    #[test]
    fn a_clip_that_fits_nowhere_falls_back_to_the_live_words() {
        let mut c = Corrections::default();
        let mut raw = spoken(0.0, "this is a longer test of voice");
        raw.extend(spoken(2.4, "quiet test"));
        c.start(2.3);
        c.end(3.0);
        c.overlay(raw, 0.0);
        c.clips = vec![FinalClip {
            t0: 2.3,
            t1: 3.0,
            text: "Client's Pest".into(),
            cut: true,
        }];
        let pasted = c.apply_final("This is a longer test of voice.");
        assert_eq!(pasted, "This is a longer quiet test of voice.");
        assert_eq!(c.note.and_then(|n| n.dropped), None);
    }

    #[test]
    fn the_corrections_made_are_kept_for_the_log() {
        let mut c = Corrections::default();
        let mut raw = spoken(0.0, "this is a glim bot test");
        raw.extend(spoken(2.4, "GlimBot"));
        c.start(2.3);
        c.end(3.0);
        c.overlay(raw, 0.0);
        let pasted = c.apply_final("This is a glim bot test. GlimBot.");
        assert_eq!(pasted, "This is a GlimBot test.");
        let [entry] = c.applied.as_slice() else {
            panic!("{:?}", c.applied);
        };
        assert_eq!(
            (entry.wrong.as_str(), entry.right.as_str()),
            ("glim bot", "GlimBot")
        );
        assert_eq!(entry.live, "this is a glim bot test GlimBot");
        assert_eq!(entry.final_text, "This is a glim bot test. GlimBot.");
        assert_eq!(entry.pasted, pasted);
        assert_eq!((entry.t0, entry.t1), (2.3, 3.0));
    }

    const SPELLED: &str = "spell capital golf lima india mike bravo oscar tango";

    /// "ask the glim bot for the logs", then a hold spelling "Glimbot".
    fn spelled_hold() -> Corrections {
        let mut c = Corrections::default();
        let mut raw = spoken(0.0, "ask the glim bot for the logs");
        c.start(2.0);
        raw.extend(spoken(2.1, SPELLED));
        c.overlay(raw.clone(), 0.0);
        let view = c.view().unwrap();
        assert_eq!(view.spelling.as_deref(), Some("Glimbot"));
        c.end(4.5);
        c.overlay(raw, 4.5 + SETTLE);
        c
    }

    #[test]
    fn a_spelled_word_replaces_the_words_it_sounds_like() {
        let mut c = spelled_hold();
        let shown: Vec<String> = c.current().into_iter().map(|(w, _)| w.text).collect();
        assert_eq!(shown.join(" "), "ask the Glimbot for the logs");
        let note = c.view().and_then(|v| v.note).unwrap();
        assert_eq!(
            (note.from.as_str(), note.to.as_str()),
            ("glim bot", "Glimbot")
        );
        assert_eq!(note.dropped, None);
        // The live words as the final text: the letters said go.
        assert_eq!(
            c.apply_final(&format!("Ask the glim bot for the logs. {SPELLED}.")),
            "Ask the Glimbot for the logs."
        );
        assert!(c.applied[0].spelled);
    }

    #[test]
    fn a_spelled_word_ignores_the_final_engines_clip() {
        let mut c = spelled_hold();
        c.clips = vec![FinalClip {
            t0: 2.0,
            t1: 4.5,
            text: "Spell capital golf, Lima, India, Mike.".into(),
            cut: true,
        }];
        assert_eq!(
            c.apply_final("Ask the glim bot for the logs."),
            "Ask the Glimbot for the logs."
        );
    }

    #[test]
    fn a_spelling_with_a_stray_word_is_dropped() {
        let mut c = Corrections::default();
        let mut raw = spoken(0.0, "ask the glim bot for the logs");
        c.start(2.0);
        raw.extend(spoken(2.1, "spell golf banana lima"));
        c.end(3.5);
        c.overlay(raw, 3.5 + SETTLE);
        let note = c.view().and_then(|v| v.note).unwrap();
        assert_eq!(note.dropped.as_deref(), Some("not letters"));
    }

    #[test]
    fn a_correction_missing_from_the_final_text_is_noted() {
        let mut c = Corrections::default();
        let mut raw = spoken(0.0, "this is a longer test of voice");
        raw.extend(spoken(2.4, "quiet test"));
        c.start(2.3);
        c.end(3.0);
        c.overlay(raw, 3.0 + SETTLE);
        let live = c.view().and_then(|v| v.note).expect("a live note");
        assert_eq!(live.dropped, None);
        assert_eq!(
            c.apply_final("Something else entirely."),
            "Something else entirely."
        );
        let note = c.view().and_then(|v| v.note).expect("a final note");
        assert!(note.id > live.id);
        assert_eq!(note.dropped.as_deref(), Some("not in the final text"));
    }

    #[test]
    fn specs_leave_an_open_hold_without_an_end() {
        let mut c = Corrections::default();
        c.start(1.0);
        assert_eq!(c.specs(), [WindowSpec { t0: 1.0, t1: None }]);
        c.end(2.0);
        assert_eq!(
            c.specs(),
            [WindowSpec {
                t0: 1.0,
                t1: Some(2.0)
            }]
        );
    }

    #[test]
    fn a_final_text_without_corrections_is_unchanged() {
        let mut c = Corrections::default();
        c.overlay(spoken(0.0, "hello  there"), 0.0);
        assert_eq!(c.apply_final("Hello,  there.\n"), "Hello,  there.\n");
        c.start(1.0);
        c.cancel();
        assert_eq!(c.apply_final("Hello,  there.\n"), "Hello,  there.\n");
    }

    #[test]
    fn a_correction_never_replaces_frozen_words() {
        let lists = steps(&[
            (0.0, "use franken term here"),
            (1.2, "and then more"),
            (3.0, "FrankenTerm"),
            (3.6, "done"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.overlay(lists[1].clone(), t);
        // A keyboard edit froze everything before 2.0 s.
        c.freeze(2.0);
        c.start(2.9);
        c.overlay(lists[2].clone(), t);
        c.end(3.4);
        let shown = c.overlay(lists[3].clone(), t);
        assert_eq!(text(&shown), "use franken term here and then more done");
        let note = c.view().unwrap().note.unwrap();
        assert_eq!(note.dropped.as_deref(), Some("frozen text"));
        assert_eq!(
            c.apply_final("Use franken term here and then more done."),
            "Use franken term here and then more done."
        );
    }

    #[test]
    fn a_correction_skips_a_frozen_match_for_an_open_one() {
        // "franken term" twice: once frozen, once after the resume. Without
        // the freeze two places fit; with it, only the open one does.
        let lists = steps(&[
            (0.0, "use franken term"),
            (1.0, "then franken term again"),
            (3.0, "FrankenTerm"),
            (3.6, "done"),
        ]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.overlay(lists[1].clone(), t);
        c.freeze(0.9);
        c.start(2.9);
        c.overlay(lists[2].clone(), t);
        c.end(3.4);
        let shown = c.overlay(lists[3].clone(), t);
        assert_eq!(text(&shown), "use franken term then FrankenTerm again done");
    }

    #[test]
    fn a_hold_that_ended_before_the_freeze_is_placed_by_it() {
        let lists = steps(&[(0.0, "use franken term here"), (1.3, "FrankenTerm")]);
        let t = 0.0;
        let mut c = Corrections::default();
        c.overlay(lists[0].clone(), t);
        c.start(1.2);
        c.overlay(lists[1].clone(), t);
        c.end(2.0);
        // The edit tap came before the live model heard past the hold.
        c.freeze(2.0);
        assert_eq!(text(&c.current()), "use FrankenTerm here");
        assert_eq!(c.view().unwrap().note.unwrap().dropped, None);
    }

    /// R21: random dictations with random holds and a freeze at a random
    /// time. No correction placed after the freeze replaces a word spoken
    /// before it, so the shown words before the boundary only change when
    /// the live model revises them.
    #[test]
    fn corrections_after_a_freeze_leave_frozen_words_alone() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let vocab = [
            "use",
            "franken",
            "term",
            "here",
            "the",
            "fox",
            "FrankenTerm",
            "run",
        ];
        for _ in 0..300 {
            let mut words: Vec<Word> = Vec::new();
            let mut c = Corrections::default();
            let mut at = 0.0f32;
            let freeze_after = rand(20);
            let mut frozen: Option<(f32, Vec<String>)> = None;
            for step in 0..30 {
                match rand(6) {
                    0 if frozen.is_none() || rand(2) == 0 => {
                        c.start(at);
                    }
                    1 => c.end(at),
                    _ => {}
                }
                for _ in 0..=rand(2) {
                    let w = vocab[rand(vocab.len() as u64) as usize];
                    words.extend(spoken(at, w));
                    at += 0.25;
                }
                let shown = c.overlay(words.clone(), at);
                if step == freeze_after {
                    c.freeze(at);
                    let kept = c
                        .current()
                        .iter()
                        .filter(|(w, _)| middle(w) < at)
                        .map(|(w, _)| w.text.clone())
                        .collect();
                    frozen = Some((at, kept));
                } else if let Some((f, kept)) = &frozen {
                    let now: Vec<String> = shown
                        .iter()
                        .filter(|(w, _)| middle(w) < *f)
                        .map(|(w, _)| w.text.clone())
                        .collect();
                    assert_eq!(&now, kept, "a frozen word changed at step {step}");
                }
            }
        }
    }
}
