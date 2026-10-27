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
//!  "status": {"phase": "finishing", "fraction": 0.4, "text": "Finishing: ..."},
//!  "target": {"app": "org.kde.konsole"}}
//! ```
//!
//! `rev` counts the preview updates of this dictation. A word's `since` is
//! the `rev` at which it got its current text and `changes` how many times
//! its text changed, so `rev - since` is how many updates it has stayed the
//! same. Word times are seconds into the recording; `start`, `end` and
//! `conf` are null when the engine gives no timing. `status` is null when
//! there is none; while finishing, `fraction` is the share done (0 to 1), or
//! null when unknown. `target` is null until the app is known.
//!
//! The file is written to a temporary name and renamed, so a reader never
//! sees half of it. It is removed when the dictation ends.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// File format version.
pub const LIVE_VERSION: u32 = 1;

/// `preview.json` in the runtime folder.
pub fn path() -> PathBuf {
    crate::config::Config::runtime_dir().join("preview.json")
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveDoc {
    pub version: u32,
    pub rev: u32,
    pub text: String,
    pub words: Vec<LiveWord>,
    pub status: Option<LiveStatus>,
    pub target: Option<LiveTarget>,
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
        let text = text.trim();
        self.version = LIVE_VERSION;
        self.rev += 1;
        let timed = timed.filter(|t| {
            t.iter()
                .map(|w| w.text.as_str())
                .eq(text.split_whitespace())
        });
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
}

static LIVE: Mutex<Live> = Mutex::new(Live {
    doc: LiveDoc {
        version: LIVE_VERSION,
        rev: 0,
        text: String::new(),
        words: Vec::new(),
        status: None,
        target: None,
    },
    session: 0,
    timed: None,
});

fn with_live(f: impl FnOnce(&mut Live)) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut live);
    write(&live.doc);
}

fn write(doc: &LiveDoc) {
    let path = path();
    let tmp = path.with_extension("json.tmp");
    let Ok(json) = serde_json::to_string(doc) else {
        return;
    };
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The engine's word timing for the preview text that follows.
pub fn timed_words(words: Vec<TimedWord>) {
    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    live.timed = Some(words);
}

/// A new preview text. On the first one of a dictation, looks up the app
/// that will receive the text on another thread.
pub fn preview(text: &str) {
    let mut first = None;
    with_live(|live| {
        let timed = live.timed.take();
        live.doc.apply_preview(text, timed.as_deref());
        first = (live.doc.rev == 1).then_some(live.session);
    });
    if let Some(session) = first {
        std::thread::spawn(move || {
            if let Some(app) = crate::osd::target::focused_app() {
                target(session, app);
            }
        });
    }
}

pub fn status(phase: Phase, fraction: Option<f64>, text: &str) {
    with_live(|live| {
        live.doc.status = Some(LiveStatus {
            phase,
            fraction: fraction.map(|f| f.clamp(0.0, 1.0)),
            text: text.to_string(),
        })
    });
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
    live.session += 1;
    let _ = std::fs::remove_file(path());
}

/// The current file, for a popup. None when there is none or it does not
/// parse.
pub fn read() -> Option<LiveDoc> {
    let json = std::fs::read_to_string(path()).ok()?;
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
        });
        let v: serde_json::Value = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["text"], "hi");
        assert_eq!(v["words"][0]["since"], 1);
        assert!(v["words"][0]["start"].is_null());
        assert_eq!(v["status"]["phase"], "finishing");
        assert_eq!(v["status"]["fraction"], 0.5);
        assert!(v["target"].is_null());
        let back: LiveDoc = serde_json::from_value(v).unwrap();
        assert_eq!(back, doc);
    }
}
