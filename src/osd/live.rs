//! The live preview as JSON, for popups that animate words.
//!
//! The daemon writes `preview.json` in the runtime folder next to the plain
//! `preview` and `preview-status` files, which stay as they are. It holds the
//! running text, its words with their audio timing and confidence when the
//! engine gives them, how long each word has kept its text, the loading or
//! finishing status, and the app that will receive the text.
//!
//! ```json
//! {"version": 1, "rev": 7, "text": "hello world",
//!  "words": [{"text": "hello", "start": 0.32, "end": 0.56, "conf": 0.98,
//!             "since": 3, "changes": 1}, ...],
//!  "status": {"phase": "finishing", "fraction": 0.4, "text": "Finishing: ...",
//!             "audio_left_s": 4.0, "eta_s": 0.6},
//!  "target": {"app": "org.kde.konsole"},
//!  "meta": {"engine": "cohere", "model": "...", "live_model": "...",
//!           "mic": "default", "language": "en"}}
//! ```
//!
//! `rev` counts the preview updates of this dictation. A word's `since` is
//! the `rev` at which it got its current text and `changes` how many times
//! its text changed, so `rev - since` is how many updates it has stayed the
//! same. Word times are seconds into the recording; `start`, `end` and
//! `conf` are null when the engine gives no timing. `status` is null when
//! there is none; while finishing, `fraction` is the share done (0 to 1), or
//! null when unknown. While finishing, `audio_left_s` is the seconds of
//! audio still to transcribe and `eta_s` the estimated seconds until the
//! final text; both are null when unknown. `target` is null until the app is
//! known. `meta` says, from the config, which engine and model make the
//! final text, which model makes the live text, the microphone, and the
//! language (null for automatic); it is written when the recording starts.
//!
//! A word may also carry `"detail": {"entropy": 0.4, "margin": 0.9,
//! "runner_up": " the"}` when the live model gives it (see [`WordDetail`]);
//! it is left out otherwise.
//!
//! While the user makes a spoken correction, the doc also carries
//! `"correction": {"hearing": "FrankenTerm", "pending": false, "note": null}`
//! ([`crate::osd::correction::CorrectionView`]) and its words already have
//! the corrections made; it is left out of dictations without one.
//!
//! The live model's own signals go to a second file, `preview-signals.json`
//! ([`ModelSignals`]), written once per streaming step. It never changes
//! `preview.json`, so the text and its words are the same with or without
//! it. Popups that do not know it never read it, and daemons from before it
//! never write it.
//!
//! ```json
//! {"version": 1, "step": 12, "audio_s": 6.72, "frames_total": 84,
//!  "frames": [{"t": 6.08, "level": -3.2,
//!              "top": [{"text": " the", "logp": -9.1}, ...]}, ...],
//!  "tail": [{"text": " of", "start": 1.28, "end": 1.36, "edge": 5,
//!            "conf": 0.64, "detail": {"entropy": 1.1, "margin": 0.3,
//!                                     "runner_up": " on"}}, ...],
//!  "with_tail": "this is a longer test of"}
//! ```
//!
//! `step` counts the live model's steps in this dictation (one per 0.56 s of
//! audio by default) and `audio_s` is the audio it had heard. `frames` holds
//! the newest [`SIGNALS_KEEP_FRAMES`] encoder frames (80 ms each) that ended
//! without a new token, oldest first, and `frames_total` how many there were
//! in all: `t` is the frame's start in seconds into the recording, `level`
//! is log10(1 - P(blank)) (0 when a token was nearly emitted, -5 for
//! silence), and `top` the likeliest tokens there with their natural log
//! probability. `tail` is what the right-context audio (the newest 0.56 s)
//! decodes to: tentative tokens that are never typed and are replaced at the
//! next step, each with `edge`, the frames between it and the newest audio.
//! A token that starts with a space starts a word. `with_tail` is the
//! committed live text followed by the tail.
//!
//! With the sound strip on (`sound-strip` in `[osd] motion`), the file also
//! has `sounds`, the newest committed frames' likeliest speech sounds
//! (`{"t": 6.08, "top": [{"text": "AH", "logp": -0.2}, ...]}`, ARPAbet,
//! `sil` for silence), and `sound_tail`, the same for the lookahead frames.
//! Without it both are left out, and the file is as before.
//!
//! Both files are written to a temporary name and renamed, so a reader never
//! sees half of one. They are removed when the dictation ends.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// File format version.
pub const LIVE_VERSION: u32 = 1;

/// `preview-signals.json` format version.
pub const SIGNALS_VERSION: u32 = 1;

/// Frames kept in `preview-signals.json`: 5.6 s, one live model window.
pub const SIGNALS_KEEP_FRAMES: usize = 70;

/// `preview.json` in the runtime folder.
pub fn path() -> PathBuf {
    crate::config::Config::runtime_dir().join("preview.json")
}

/// `preview-signals.json` in the runtime folder.
pub fn signals_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("preview-signals.json")
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveDoc {
    pub version: u32,
    pub rev: u32,
    pub text: String,
    pub words: Vec<LiveWord>,
    pub status: Option<LiveStatus>,
    pub target: Option<LiveTarget>,
    #[serde(default)]
    pub meta: Option<LiveMeta>,
    /// Spoken corrections ([`crate::osd::correction`]); left out when the
    /// dictation has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<crate::osd::correction::CorrectionView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveWord {
    pub text: String,
    /// Seconds into the recording.
    pub start: Option<f32>,
    pub end: Option<f32>,
    /// The engine's confidence, 0 to 1.
    pub conf: Option<f32>,
    /// The `rev` at which the word got its current text.
    pub since: u32,
    /// How many times the word's text changed.
    pub changes: u32,
    /// The live model's distribution at the word, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<WordDetail>,
}

/// How sure the live model was of a word or token, from its distribution
/// over the vocabulary. For a word, its least certain token's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordDetail {
    /// Shannon entropy in nats: 0 when one token had all the probability.
    pub entropy: f32,
    /// Probability of the chosen token minus that of the runner-up.
    pub margin: f32,
    /// The second likeliest token (a leading space starts a word), or None
    /// when that was blank.
    pub runner_up: Option<String>,
}

impl WordDetail {
    /// The less certain of two: the higher entropy, and the smaller margin
    /// with its runner-up.
    pub fn least_certain(self, other: WordDetail) -> WordDetail {
        let entropy = self.entropy.max(other.entropy);
        let mut out = if other.margin < self.margin {
            other
        } else {
            self
        };
        out.entropy = entropy;
        out
    }
}

/// One line of the live worker's word list: a timed word and, from newer
/// workers, its detail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordLine {
    #[serde(flatten)]
    pub word: TimedWord,
    #[serde(default)]
    pub detail: Option<WordDetail>,
}

/// One encoder frame (80 ms) that ended without a new token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameLevel {
    /// Start of the frame, seconds into the recording.
    pub t: f32,
    /// log10(1 - P(blank)): near 0 when a token almost came, about -5 in
    /// silence.
    pub level: f32,
    /// The likeliest tokens, likeliest first.
    pub top: Vec<Guess>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guess {
    pub text: String,
    /// Natural log probability.
    pub logp: f32,
}

/// The speech sounds the sound strip's probe hears in one encoder frame
/// (80 ms).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundFrame {
    /// Start of the frame, seconds into the recording.
    pub t: f32,
    /// The likeliest phones (ARPAbet, `sil` for silence), likeliest first,
    /// with their natural log probability.
    pub top: Vec<Guess>,
}

/// A tentative token decoded from the newest audio. Never typed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TailToken {
    pub text: String,
    /// Seconds into the recording.
    pub start: f32,
    pub end: f32,
    /// Encoder frames between this token and the newest audio.
    pub edge: u32,
    /// The model's probability for the token, 0 to 1.
    pub conf: f32,
    pub detail: Option<WordDetail>,
}

/// One streaming step of the live model, as the worker sends it: the frames
/// new since the last step and the current tail.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalsStep {
    pub step: u32,
    pub audio_s: f32,
    pub frames: Vec<FrameLevel>,
    pub tail: Vec<TailToken>,
    pub with_tail: String,
    /// With the sound strip on: the sounds of the frames this step
    /// committed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<SoundFrame>,
    /// With the sound strip on: the sounds of the lookahead frames after
    /// them, heard again by the next step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sound_tail: Vec<SoundFrame>,
}

/// `preview-signals.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelSignals {
    pub version: u32,
    pub step: u32,
    pub audio_s: f32,
    pub frames_total: u64,
    pub frames: Vec<FrameLevel>,
    pub tail: Vec<TailToken>,
    pub with_tail: String,
    /// With the sound strip on: the newest [`SIGNALS_KEEP_FRAMES`] committed
    /// frames' sounds, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<SoundFrame>,
    /// With the sound strip on: the lookahead frames' sounds of the last
    /// step or tail tick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sound_tail: Vec<SoundFrame>,
}

impl ModelSignals {
    /// Take in one step: append its frames, keeping the newest
    /// [`SIGNALS_KEEP_FRAMES`], and replace the tail.
    pub fn apply(&mut self, step: SignalsStep) {
        self.version = SIGNALS_VERSION;
        self.step = step.step;
        self.audio_s = step.audio_s;
        self.frames_total += step.frames.len() as u64;
        self.frames.extend(step.frames);
        let excess = self.frames.len().saturating_sub(SIGNALS_KEEP_FRAMES);
        self.frames.drain(..excess);
        self.tail = step.tail;
        self.with_tail = step.with_tail;
        self.sounds.extend(step.sounds);
        let excess = self.sounds.len().saturating_sub(SIGNALS_KEEP_FRAMES);
        self.sounds.drain(..excess);
        self.sound_tail = step.sound_tail;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// The live model is loading.
    Loading,
    /// The recording stopped and the final text is being made.
    Finishing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveStatus {
    pub phase: Phase,
    /// Share done, 0 to 1, or None when unknown.
    pub fraction: Option<f64>,
    pub text: String,
    /// Seconds of audio still to transcribe, while finishing.
    #[serde(default)]
    pub audio_left_s: Option<f32>,
    /// Estimated seconds until the final text, while finishing.
    #[serde(default)]
    pub eta_s: Option<f32>,
}

/// What makes the text, from the config.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveMeta {
    /// The engine of the final text, such as "whisper" or "cohere".
    pub engine: String,
    /// Its model name or path.
    pub model: String,
    /// The model of the live text, if a separate one makes it.
    pub live_model: Option<String>,
    /// The audio input device as configured, "default" for the default.
    pub mic: String,
    /// The language code, or null for automatic detection.
    pub language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveTarget {
    /// The app id or window class of the focused window.
    pub app: String,
}

/// A word with its audio timing, from the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedWord {
    pub text: String,
    pub start: f32,
    pub end: f32,
    pub conf: Option<f32>,
}

impl LiveDoc {
    /// A new preview text. `timed` gives the words' timing when it belongs
    /// to this text (its words joined with spaces are the text); otherwise
    /// the words are the text split on whitespace, without timing.
    pub fn apply_preview(&mut self, text: &str, timed: Option<&[TimedWord]>) {
        self.apply_preview_with_details(text, timed, None);
    }

    /// [`Self::apply_preview`] with a detail for each timed word. The details
    /// are used only together with the timing, for the same text.
    pub fn apply_preview_with_details(
        &mut self,
        text: &str,
        timed: Option<&[TimedWord]>,
        details: Option<&[Option<WordDetail>]>,
    ) {
        let text = text.trim();
        self.version = LIVE_VERSION;
        self.rev += 1;
        let timed = timed.filter(|t| {
            t.iter()
                .map(|w| w.text.as_str())
                .eq(text.split_whitespace())
        });
        let details = details.filter(|d| Some(d.len()) == timed.map(|t| t.len()));
        let new: Vec<(&str, Option<&TimedWord>)> = match timed {
            Some(t) => t.iter().map(|w| (w.text.as_str(), Some(w))).collect(),
            None => text.split_whitespace().map(|w| (w, None)).collect(),
        };
        let rev = self.rev;
        let words = new
            .into_iter()
            .enumerate()
            .map(|(i, (word, timing))| {
                let (since, changes) = match self.words.get(i) {
                    Some(old) if old.text == word => (old.since, old.changes),
                    Some(old) => (rev, old.changes + 1),
                    None => (rev, 0),
                };
                LiveWord {
                    text: word.to_string(),
                    start: timing.map(|t| t.start),
                    end: timing.map(|t| t.end),
                    conf: timing.and_then(|t| t.conf),
                    since,
                    changes,
                    detail: details.and_then(|d| d[i].clone()),
                }
            })
            .collect();
        self.words = words;
        self.text = text.to_string();
    }
}

struct Live {
    doc: LiveDoc,
    /// Counts dictations, so a late answer about one does not land in the next.
    session: u64,
    /// Timing for the next preview, and the text it belongs to.
    timed: Option<Vec<TimedWord>>,
    /// The words' details, beside `timed`.
    details: Option<Vec<Option<WordDetail>>>,
    /// The newest preview as the live model gave it, before corrections.
    raw: Option<RawPreview>,
    signals: ModelSignals,
}

type RawPreview = (
    String,
    Option<Vec<TimedWord>>,
    Option<Vec<Option<WordDetail>>>,
);

static LIVE: Mutex<Live> = Mutex::new(Live {
    doc: LiveDoc {
        version: LIVE_VERSION,
        rev: 0,
        text: String::new(),
        words: Vec::new(),
        status: None,
        target: None,
        meta: None,
        correction: None,
    },
    session: 0,
    timed: None,
    details: None,
    raw: None,
    signals: ModelSignals {
        version: SIGNALS_VERSION,
        step: 0,
        audio_s: 0.0,
        frames_total: 0,
        frames: Vec::new(),
        tail: Vec::new(),
        with_tail: String::new(),
        sounds: Vec::new(),
        sound_tail: Vec::new(),
    },
});

fn with_live(f: impl FnOnce(&mut Live)) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut live);
    write(&live.doc);
}

fn write(doc: &LiveDoc) {
    write_json(&path(), doc);
}

fn write_json(path: &std::path::Path, value: &impl Serialize) {
    let tmp = path.with_extension("json.tmp");
    let Ok(json) = serde_json::to_string(value) else {
        return;
    };
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The engine's word timing for the preview text that follows.
pub fn timed_words(words: Vec<TimedWord>) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    live.timed = Some(words);
    live.details = None;
}

/// The live worker's word list for the preview text that follows: timing,
/// and each word's detail when the worker gives it.
pub fn word_lines(lines: Vec<WordLine>) {
    let (words, details): (Vec<_>, Vec<_>) = lines.into_iter().map(|l| (l.word, l.detail)).unzip();
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    live.timed = Some(words);
    live.details = details.iter().any(Option::is_some).then_some(details);
}

/// One streaming step of the live model: written to `preview-signals.json`.
pub fn signals(step: SignalsStep) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    live.signals.apply(step);
    write_json(&signals_path(), &live.signals);
}

/// A new preview text. On the first one of a dictation, looks up the app
/// that will receive the text on another thread.
///
/// Returns the text to show: `text` itself, or with the spoken corrections
/// made when the dictation has any.
pub fn preview(text: &str) -> String {
    let mut first = None;
    let mut shown = text.to_string();
    with_live(|live| {
        let timed = live.timed.take();
        let details = live.details.take();
        live.raw = Some((text.to_string(), timed, details));
        render(live, true);
        if live.doc.correction.is_some() {
            shown = live.doc.text.clone();
        }
        first = (live.doc.rev == 1).then_some(live.session);
    });
    if let Some(session) = first {
        std::thread::spawn(move || {
            if let Some(app) = crate::osd::target::focused_app() {
                target(session, app);
            }
        });
    }
    shown
}

/// Puts the newest preview in the doc with the spoken corrections made.
/// `fresh`: it is a new preview, so corrections waiting for it are placed.
/// Without timed words the text goes in as it is.
fn render(live: &mut Live, fresh: bool) {
    use crate::osd::correction;
    let Some((text, timed, details)) = live.raw.clone() else {
        live.doc.correction = correction::view();
        return;
    };
    let matching = timed.as_ref().filter(|t| {
        t.iter()
            .map(|w| w.text.as_str())
            .eq(text.split_whitespace())
    });
    if let Some(words) = matching {
        let n = words.len();
        let details = details.clone().filter(|d| d.len() == n);
        let raw = words
            .iter()
            .cloned()
            .zip(details.unwrap_or_else(|| vec![None; n]))
            .collect();
        let shown = if fresh {
            correction::overlay(raw)
        } else {
            correction::current()
        };
        if correction::active() {
            let (words, details): (Vec<_>, Vec<_>) = shown.into_iter().unzip();
            let text = words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            live.doc
                .apply_preview_with_details(&text, Some(&words), Some(&details));
            live.doc.correction = correction::view();
            return;
        }
    }
    live.doc
        .apply_preview_with_details(&text, timed.as_deref(), details.as_deref());
    live.doc.correction = None;
}

/// A spoken correction started, ended or was dropped: show the text and
/// the correction afresh. Returns the text to show, None before the first
/// preview.
pub fn refresh_correction() -> Option<String> {
    let mut shown = None;
    with_live(|live| {
        render(live, false);
        shown = live.raw.is_some().then(|| live.doc.text.clone());
    });
    shown
}

pub fn status(phase: Phase, fraction: Option<f64>, text: &str) {
    finishing_status(phase, fraction, text, None, None);
}

/// A status with the seconds of audio left and the estimated seconds until
/// the final text, when they are known.
pub fn finishing_status(
    phase: Phase,
    fraction: Option<f64>,
    text: &str,
    audio_left_s: Option<f32>,
    eta_s: Option<f32>,
) {
    with_live(|live| {
        live.doc.status = Some(LiveStatus {
            phase,
            fraction: fraction.map(|f| f.clamp(0.0, 1.0)),
            text: text.to_string(),
            audio_left_s,
            eta_s,
        })
    });
}

/// What makes the text, at the start of a recording.
pub fn meta(meta: LiveMeta) {
    with_live(|live| live.doc.meta = Some(meta));
}

pub fn clear_status() {
    with_live(|live| live.doc.status = None);
}

/// The app that will receive the text of dictation `session`.
fn target(session: u64, app: String) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if live.session == session && live.doc.rev > 0 {
        live.doc.target = Some(LiveTarget { app });
        write(&live.doc);
    }
}

/// The dictation ended: forget it and remove the file.
pub fn clear() {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    live.doc = LiveDoc {
        version: LIVE_VERSION,
        ..LiveDoc::default()
    };
    live.timed = None;
    live.details = None;
    live.raw = None;
    crate::osd::correction::clear();
    live.signals = ModelSignals {
        version: SIGNALS_VERSION,
        ..ModelSignals::default()
    };
    live.session += 1;
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_file(signals_path());
}

/// The current file, for a popup. None when there is none or it does not
/// parse.
pub fn read() -> Option<LiveDoc> {
    let json = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&json).ok()
}

/// The current `preview-signals.json`, for a popup. None when there is none
/// or it does not parse.
pub fn read_signals() -> Option<ModelSignals> {
    let json = std::fs::read_to_string(signals_path()).ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(words: &[(&str, f32)]) -> Vec<TimedWord> {
        words
            .iter()
            .map(|&(text, start)| TimedWord {
                text: text.into(),
                start,
                end: start + 0.2,
                conf: Some(0.9),
            })
            .collect()
    }

    #[test]
    fn words_keep_their_rev_until_their_text_changes() {
        let mut doc = LiveDoc::default();
        doc.apply_preview("hello", None);
        doc.apply_preview("hello wor", None);
        doc.apply_preview("hello world", None);
        let w: Vec<_> = doc
            .words
            .iter()
            .map(|w| (w.text.as_str(), w.since, w.changes))
            .collect();
        assert_eq!(doc.rev, 3);
        assert_eq!(w, [("hello", 1, 0), ("world", 3, 1)]);
    }

    #[test]
    fn timing_is_used_only_for_its_own_text() {
        let mut doc = LiveDoc::default();
        let t = timed(&[("hello", 0.1), ("world", 0.5)]);
        doc.apply_preview("hello world", Some(&t));
        assert_eq!(doc.words[1].start, Some(0.5));
        assert_eq!(doc.words[1].conf, Some(0.9));
        doc.apply_preview("hello world again", Some(&t));
        assert!(doc
            .words
            .iter()
            .all(|w| w.start.is_none() && w.conf.is_none()));
        assert_eq!(doc.words[1].since, 1);
    }

    #[test]
    fn the_json_has_the_documented_shape() {
        let mut doc = LiveDoc::default();
        doc.apply_preview(" hi ", None);
        doc.status = Some(LiveStatus {
            phase: Phase::Finishing,
            fraction: Some(0.5),
            text: "Finishing".into(),
            audio_left_s: Some(4.0),
            eta_s: None,
        });
        doc.meta = Some(LiveMeta {
            engine: "cohere".into(),
            model: "m".into(),
            live_model: None,
            mic: "default".into(),
            language: Some("en".into()),
        });
        let v: serde_json::Value = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["text"], "hi");
        assert_eq!(v["words"][0]["since"], 1);
        assert!(v["words"][0]["start"].is_null());
        assert_eq!(v["status"]["phase"], "finishing");
        assert_eq!(v["status"]["fraction"], 0.5);
        assert_eq!(v["status"]["audio_left_s"], 4.0);
        assert!(v["status"]["eta_s"].is_null());
        assert!(v["target"].is_null());
        assert_eq!(v["meta"]["engine"], "cohere");
        assert_eq!(v["meta"]["language"], "en");
        let back: LiveDoc = serde_json::from_value(v).unwrap();
        assert_eq!(back, doc);
    }

    fn detail(margin: f32) -> Option<WordDetail> {
        Some(WordDetail {
            entropy: 0.5,
            margin,
            runner_up: Some(" the".into()),
        })
    }

    #[test]
    fn details_go_with_their_own_timing_only() {
        let mut doc = LiveDoc::default();
        let t = timed(&[("hello", 0.1), ("world", 0.5)]);
        let d = [detail(0.9), None];
        doc.apply_preview_with_details("hello world", Some(&t), Some(&d));
        assert_eq!(doc.words[0].detail, detail(0.9));
        assert_eq!(doc.words[1].detail, None);
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["words"][0]["detail"]["runner_up"], " the");
        assert!(v["words"][1].get("detail").is_none());
        // A text the timing does not describe gets neither.
        doc.apply_preview_with_details("hello there", Some(&t), Some(&d));
        assert!(doc.words.iter().all(|w| w.detail.is_none()));
    }

    /// The worker's word lines parse with and without details, so the text
    /// and timing stay the same either way.
    #[test]
    fn word_lines_parse_old_and_new_shapes() {
        let old = r#"[{"text":"hi","start":0.1,"end":0.3,"conf":0.9}]"#;
        let new = r#"[{"text":"hi","start":0.1,"end":0.3,"conf":0.9,
            "detail":{"entropy":0.2,"margin":0.8,"runner_up":null}}]"#;
        let old: Vec<WordLine> = serde_json::from_str(old).unwrap();
        let new: Vec<WordLine> = serde_json::from_str(new).unwrap();
        assert_eq!(old[0].word, new[0].word);
        assert_eq!(old[0].detail, None);
        assert_eq!(new[0].detail.as_ref().unwrap().margin, 0.8);
    }

    #[test]
    fn signals_keep_the_newest_frames_and_replace_the_tail() {
        let frame = |t: f32| FrameLevel {
            t,
            level: -3.0,
            top: vec![Guess {
                text: " a".into(),
                logp: -9.0,
            }],
        };
        let tail = |text: &str| TailToken {
            text: text.into(),
            start: 1.0,
            end: 1.08,
            edge: 2,
            conf: 0.6,
            detail: None,
        };
        let mut s = ModelSignals::default();
        s.apply(SignalsStep {
            step: 1,
            audio_s: 1.12,
            frames: (0..50).map(|i| frame(i as f32 * 0.08)).collect(),
            tail: vec![tail(" on")],
            with_tail: "test on".into(),
            ..SignalsStep::default()
        });
        s.apply(SignalsStep {
            step: 2,
            audio_s: 1.68,
            frames: (50..57).map(|i| frame(i as f32 * 0.08)).collect(),
            tail: vec![tail(" of")],
            with_tail: "test of".into(),
            ..SignalsStep::default()
        });
        assert_eq!(s.version, SIGNALS_VERSION);
        assert_eq!(s.frames_total, 57);
        assert_eq!(s.frames.len(), 57);
        s.apply(SignalsStep {
            step: 3,
            frames: (57..80).map(|i| frame(i as f32 * 0.08)).collect(),
            ..SignalsStep::default()
        });
        assert_eq!(s.frames.len(), SIGNALS_KEEP_FRAMES);
        assert_eq!(s.frames_total, 80);
        assert!((s.frames[0].t - 0.8).abs() < 1e-6);
        assert!(s.tail.is_empty());
        let back: ModelSignals = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Without the sound strip the file has no sound keys at all.
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("sound"));
    }

    #[test]
    fn sounds_keep_the_newest_frames_and_replace_their_tail() {
        let sound = |t: f32, p: &str| SoundFrame {
            t,
            top: vec![Guess {
                text: p.into(),
                logp: -0.1,
            }],
        };
        let mut s = ModelSignals::default();
        s.apply(SignalsStep {
            step: 1,
            sounds: (0..60).map(|i| sound(i as f32 * 0.08, "AH")).collect(),
            sound_tail: vec![sound(4.8, "S")],
            ..SignalsStep::default()
        });
        s.apply(SignalsStep {
            step: 2,
            sounds: (60..77).map(|i| sound(i as f32 * 0.08, "T")).collect(),
            sound_tail: vec![sound(6.16, "IY")],
            ..SignalsStep::default()
        });
        assert_eq!(s.sounds.len(), SIGNALS_KEEP_FRAMES);
        assert!((s.sounds[0].t - 0.56).abs() < 1e-6);
        assert_eq!(s.sound_tail, [sound(6.16, "IY")]);
        // A step without sounds (a tail tick with none) clears the tail.
        s.apply(SignalsStep::default());
        assert!(s.sound_tail.is_empty());
        let back: ModelSignals = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    /// A file from before `meta`, `audio_left_s` and `eta_s` still reads.
    #[test]
    fn an_older_file_still_parses() {
        let json = r#"{"version":1,"rev":1,"text":"hi","words":[],
            "status":{"phase":"finishing","fraction":null,"text":"Finishing"},
            "target":null}"#;
        let doc: LiveDoc = serde_json::from_str(json).unwrap();
        assert_eq!(doc.status.unwrap().audio_left_s, None);
        assert_eq!(doc.meta, None);
    }
}
