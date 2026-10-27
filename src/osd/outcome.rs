//! How the last dictation ended, for a popup that is about to close.
//!
//! The daemon writes one word to `outcome` in the runtime folder when a
//! dictation ends: `done` when the text was delivered, `cancelled`, or
//! `failed`. It removes the file when the next recording starts. The state
//! file alone cannot tell these apart: a streaming dictation goes from
//! `streaming` to `idle` both when it is stopped and when it is cancelled.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The text was delivered.
    Done,
    /// The user cancelled the dictation.
    Cancelled,
    /// Recording, transcription or output failed.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Outcome> {
        match s.trim() {
            "done" => Some(Outcome::Done),
            "cancelled" => Some(Outcome::Cancelled),
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
        for o in [Outcome::Done, Outcome::Cancelled, Outcome::Failed] {
            assert_eq!(Outcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(Outcome::parse("done\n"), Some(Outcome::Done));
        assert_eq!(Outcome::parse("maybe"), None);
    }
}
