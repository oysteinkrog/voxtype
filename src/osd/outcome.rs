//! How the last dictation ended, for a popup that is about to close.
//!
//! The daemon writes one word to `outcome` in the runtime folder when a
//! dictation ends: `done` when the text was delivered, `cancelled`,
//! `empty` when no speech was heard, or `failed`. It removes the file when
//! the next recording starts. The state file alone cannot tell these apart:
//! a streaming dictation goes from `streaming` to `idle` both when it is
//! stopped and when it is cancelled.
//!
//! After a typed or pasted dictation it also writes `result.json`, before
//! `outcome`:
//!
//! ```json
//! {"text": "Hello there.", "chars": 12, "method": "wtype", "enter": false,
//!  "output_ms": 85, "latency_ms": 640, "audio_s": 3.2}
//! ```
//!
//! `method` is the output driver that delivered the text, `enter` whether
//! Enter was sent after it, `output_ms` how long typing or pasting took,
//! `latency_ms` the time from the end of the recording to the text being
//! delivered (null when unknown), and `audio_s` the seconds of audio.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The text was delivered.
    Done,
    /// The user cancelled the dictation.
    Cancelled,
    /// No speech was heard, so nothing was transcribed.
    Empty,
    /// Recording, transcription or output failed.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Cancelled => "cancelled",
            Outcome::Empty => "empty",
            Outcome::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Outcome> {
        match s.trim() {
            "done" => Some(Outcome::Done),
            "cancelled" => Some(Outcome::Cancelled),
            "empty" => Some(Outcome::Empty),
            "failed" => Some(Outcome::Failed),
            _ => None,
        }
    }
}

/// `outcome` in the runtime folder.
pub fn path() -> PathBuf {
    crate::config::Config::runtime_dir().join("outcome")
}

/// Record how the dictation ended. Written to a temporary name and renamed,
/// so a reader never sees half of it.
pub fn write(outcome: Outcome) {
    let path = path();
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, outcome.as_str()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// A recording starts: the last outcome no longer applies.
pub fn clear() {
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_file(result_path());
    *STOPPED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// What a typed or pasted dictation delivered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub text: String,
    pub chars: usize,
    /// The output driver that delivered the text, such as "wtype".
    pub method: String,
    /// Enter was sent after the text.
    pub enter: bool,
    pub output_ms: u64,
    /// From the end of the recording to the text being delivered.
    pub latency_ms: Option<u64>,
    pub audio_s: Option<f32>,
}

/// When the recording stopped, for `latency_ms`.
static STOPPED: Mutex<Option<Instant>> = Mutex::new(None);

/// The recording stopped and the final text is being made.
pub fn stopped() {
    *STOPPED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// Milliseconds since [`stopped`], if it was called for this dictation.
pub fn since_stop_ms() -> Option<u64> {
    STOPPED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|t| t.elapsed().as_millis() as u64)
}

/// `result.json` in the runtime folder.
pub fn result_path() -> PathBuf {
    crate::config::Config::runtime_dir().join("result.json")
}

pub fn write_delivery(delivery: &Delivery) {
    let path = result_path();
    let tmp = path.with_extension("json.tmp");
    let Ok(json) = serde_json::to_string(delivery) else {
        return;
    };
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The last delivery, if the file is there and parses.
pub fn read_delivery() -> Option<Delivery> {
    serde_json::from_str(&std::fs::read_to_string(result_path()).ok()?).ok()
}

/// The last outcome, if the file is there and parses.
pub fn read() -> Option<Outcome> {
    Outcome::parse(&std::fs::read_to_string(path()).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for o in [
            Outcome::Done,
            Outcome::Cancelled,
            Outcome::Empty,
            Outcome::Failed,
        ] {
            assert_eq!(Outcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(Outcome::parse("done\n"), Some(Outcome::Done));
        assert_eq!(Outcome::parse("maybe"), None);
    }

    #[test]
    fn delivery_json_has_the_documented_shape() {
        let d = Delivery {
            text: "Hello there.".into(),
            chars: 12,
            method: "wtype".into(),
            enter: false,
            output_ms: 85,
            latency_ms: None,
            audio_s: Some(3.2),
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["method"], "wtype");
        assert!(v["latency_ms"].is_null());
        assert_eq!(serde_json::from_value::<Delivery>(v).unwrap(), d);
    }
}
