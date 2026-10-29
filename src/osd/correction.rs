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
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::osd::live::{TimedWord, WordDetail};
use crate::text::align::{self, Refused};

/// How long after the key comes up a correction waits for the live model's
/// words of the hold, when no word after the hold has arrived sooner.
pub const SETTLE: Duration = Duration::from_millis(1200);

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
}

/// A live word and its detail.
pub type Word = (TimedWord, Option<WordDetail>);

#[derive(Debug, Clone, PartialEq)]
enum State {
    Open,
    /// The key came up at this moment.
    Pending(Instant),
    /// Words timed `from..=to` (`wrong`) are replaced by `right`.
    Placed {
        from: f32,
        to: f32,
        wrong: String,
        right: Vec<String>,
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

    /// The key came up `t1` seconds into the recording, at `now`.
    pub fn end(&mut self, t1: f32, now: Instant) {
        if let Some(w) = self.open() {
            w.t1 = t1.max(w.t0);
            w.state = State::Pending(now);
        }
    }

    /// Another key was pressed during the hold: its words stay dictation.
    pub fn cancel(&mut self) {
        self.windows.retain(|w| w.state != State::Open);
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
    /// Holds that ended `SETTLE` before `now`, or that the live model has
    /// heard past, are placed now.
    pub fn overlay(&mut self, raw: Vec<Word>, now: Instant) -> Vec<Word> {
        self.raw = raw;
        self.render(Some(now))
    }

    /// The words to show for the current list, placing nothing new.
    pub fn current(&mut self) -> Vec<Word> {
        self.render(None)
    }

    /// `settle_at`: place pending holds that are ready at this moment; None
    /// places none, and `force` (in [`Self::apply_final`]) places all.
    fn render(&mut self, settle_at: Option<Instant>) -> Vec<Word> {
        self.render_with(settle_at, false)
    }

    fn render_with(&mut self, settle_at: Option<Instant>, force: bool) -> Vec<Word> {
        let mut out = self.raw.clone();
        for i in 0..self.windows.len() {
            let Window { t0, t1, .. } = self.windows[i];
            out.retain(|(w, _)| !inside(w, t0, t1));
            if let State::Pending(ended) = self.windows[i].state {
                let heard_past = self.raw.iter().any(|(w, _)| w.start >= t1);
                let due = settle_at.is_some_and(|now| now >= ended + SETTLE);
                if force || (settle_at.is_some() && (heard_past || due)) {
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
        self.notes += 1;
        let mut note = CorrectionNote {
            id: self.notes,
            from: String::new(),
            to: heard.join(" "),
            dropped: None,
        };
        let placed = if heard.is_empty() {
            Err("nothing heard")
        } else {
            align::place(&texts, &heard).map_err(|r| match r {
                Refused::Empty => "nothing to replace",
                Refused::NoMatch { .. } => "no match",
                Refused::Ambiguous { .. } => "two places fit",
                Refused::TooShort { .. } => "too short to place",
            })
        };
        let state = match placed {
            Ok(span) => {
                note.from = texts[span.start..span.end].join(" ");
                State::Placed {
                    from: before[span.start].start,
                    to: before[span.end - 1].end,
                    wrong: note.from.clone(),
                    right: heard,
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
        let hearing = self
            .windows
            .iter()
            .find(|w| w.state == State::Open)
            .map(|w| {
                self.raw
                    .iter()
                    .filter(|(word, _)| inside(word, w.t0, w.t1))
                    .map(|(word, _)| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        Some(CorrectionView {
            hearing,
            pending: self
                .windows
                .iter()
                .any(|w| matches!(w.state, State::Pending(_))),
            note: self.note.clone(),
        })
    }

    /// The final text with the corrections made. Each correction replaces
    /// the words it was placed on (the newest match before the hold's
    /// start). Its words are the final engine's own text of the hold when
    /// the worker gave one ([`FinalClip`]) that differs from the replaced
    /// words, else the live words. When the hold's audio was not cut out of
    /// the final text, the engine heard the respoken words as well, so that
    /// copy (the newest match before the hold's end) goes first. The final
    /// text has no timing: the live word counts say how far to look. A
    /// text with no placed correction comes back unchanged.
    pub fn apply_final(&mut self, text: &str) -> String {
        self.render_with(None, true);
        let placed: Vec<(f32, f32, Vec<String>, bool)> = self
            .windows
            .iter()
            .filter_map(|w| match &w.state {
                State::Placed { wrong, right, .. } => {
                    let clip = self.clips.iter().find(|c| (c.t0 - w.t0).abs() < 0.05);
                    let cut = clip.is_some_and(|c| c.cut);
                    let words = clip
                        .map(|c| clip_words(&c.text, wrong))
                        .filter(|words| !same_words(&words.join(" "), wrong))
                        .unwrap_or_else(|| right.clone());
                    if same_words(&words.join(" "), wrong) {
                        tracing::info!(%wrong, "Correction heard as the same words: spell it");
                        return None;
                    }
                    Some((w.t0, w.t1, words, cut))
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
        for (t0, t1, right, cut) in placed {
            if cut {
                // The live count includes the respoken words; the text not.
                shift -= self.raw.iter().filter(|(w, _)| inside(w, t0, t1)).count() as isize;
            } else if let Ok(copy) = align::place_before(&words, before(t1, shift), &right) {
                words.drain(copy.start..copy.end);
                shift -= (copy.end - copy.start) as isize;
            }
            let hold_start = before(t0, shift);
            if let Ok(span) = align::place_before(&words, hold_start, &right) {
                shift += right.len() as isize - (span.end - span.start) as isize;
                words.splice(span.start..span.end, right);
            }
        }
        words.join(" ")
    }
}

/// Whether `a` and `b` are the same words, ignoring case and punctuation.
/// "FrankenTerm" and "franken term" are not: joining words is a fix.
fn same_words(a: &str, b: &str) -> bool {
    let key = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    key(a) == key(b)
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
/// made: what the final pass pushes down when it decodes the hold. None for
/// a hold that places nowhere.
pub fn wrong_spans(raw: Vec<Word>, holds: &[(f32, f32)]) -> Vec<Option<String>> {
    let mut c = Corrections::default();
    let now = Instant::now();
    for &(t0, t1) in holds {
        c.start(t0);
        c.end(t1, now);
    }
    c.raw = raw;
    c.render_with(None, true);
    c.windows
        .iter()
        .map(|w| match &w.state {
            State::Placed { wrong, .. } => Some(wrong.clone()),
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
        c.end(t1, Instant::now());
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

/// The dictation ended.
pub fn clear() {
    *CORRECTIONS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let _ = std::fs::remove_file(windows_path());
}

/// A new live word list, with the corrections made.
pub fn overlay(raw: Vec<Word>) -> Vec<Word> {
    with(|c| c.overlay(raw, Instant::now()))
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
        let t = Instant::now();
        let mut c = Corrections::default();
        assert_eq!(
            text(&c.overlay(lists[0].clone(), t)),
            "use franken term here"
        );

        c.start(1.2);
        let shown = c.overlay(lists[1].clone(), t);
        assert_eq!(text(&shown), "use franken term here");
        assert_eq!(c.view().unwrap().hearing.as_deref(), Some("FrankenTerm"));

        c.end(2.0, t);
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
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0, t);
        c.overlay(lists[2].clone(), t);
        // The live model revises "franken term" to "frank and term".
        let mut revised = lists[2].clone();
        revised[1].0.text = "frank".into();
        revised[2].0.text = "and-term".into();
        assert_eq!(text(&c.overlay(revised, t)), "use FrankenTerm here and");
    }

    #[test]
    fn a_hold_settles_after_the_wait_with_no_words_after_it() {
        let lists = steps(&[(0.0, "use franken term here"), (1.3, "FrankenTerm")]);
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0, t);
        assert_eq!(
            text(&c.overlay(lists[1].clone(), t)),
            "use franken term here"
        );
        let later = t + SETTLE;
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
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.7);
        c.end(2.4, t);
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
        let t = Instant::now();
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
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0, t);
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
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.2);
        c.overlay(lists[1].clone(), t);
        c.end(2.0, t);
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
        let t = Instant::now();
        let mut c = Corrections::default();
        c.start(1.2);
        c.end(2.0, t);
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
    fn specs_leave_an_open_hold_without_an_end() {
        let mut c = Corrections::default();
        c.start(1.0);
        assert_eq!(c.specs(), [WindowSpec { t0: 1.0, t1: None }]);
        c.end(2.0, Instant::now());
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
        c.overlay(spoken(0.0, "hello  there"), Instant::now());
        assert_eq!(c.apply_final("Hello,  there.\n"), "Hello,  there.\n");
        c.start(1.0);
        c.cancel();
        assert_eq!(c.apply_final("Hello,  there.\n"), "Hello,  there.\n");
    }
}
