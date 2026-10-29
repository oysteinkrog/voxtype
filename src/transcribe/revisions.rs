//! Revision log for the live preview.
//!
//! Records every change of the running preview text with its time, the
//! words with their audio timing, and the final text that was pasted. One
//! JSON Lines file per recording. It exists to measure how words change on
//! screen, so an overlay can animate them without jumps. Off unless
//! `live_preview_revisions_dir` is set.
//!
//! File format (version 1), one JSON object per line:
//! - `{"kind": "header", "version": 1, "source": "live" | "replay", ...}`
//! - `{"kind": "preview", "rev": 1, "t_ms": 812, "audio_ms": 800, "text": "...",
//!   "words": [{"text": "hello", "start": 0.32, "end": 0.56, "conf": null}]}`
//! - `{"kind": "final", "t_ms": 5120, "source": "engine", "model": "...", "text": "..."}`
//!
//! `t_ms` is wall time since the first audio block, `audio_ms` the audio
//! received so far. Word times are seconds into the recording. `conf` is
//! the model's confidence for the word (the lowest token probability in
//! it), or null when the model gives none.

use serde_json::{json, Value};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// File format version, in the header line.
pub const REVISIONS_VERSION: u32 = 1;

/// One word of a preview revision.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Word {
    pub text: String,
    /// Seconds into the recording.
    pub start: f32,
    pub end: f32,
    pub conf: Option<f32>,
}

/// Group subword tokens into words. A token that starts with a space (the
/// SentencePiece word marker, already decoded) starts a new word; a token
/// that is only the marker makes the next token start one. Unlike
/// parakeet-rs's own word grouping, this never drops repeated words, so the
/// words always join back to the text. A word's confidence is the lowest of
/// its tokens' scores, so one doubtful subword marks the whole word; None
/// if any token has no score.
pub fn words_from_tokens<'a>(
    tokens: impl IntoIterator<Item = (&'a str, f32, f32, Option<f32>)>,
) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    let mut boundary = true;
    for (text, start, end, conf) in tokens {
        boundary |= text.starts_with(' ') || text.starts_with('\u{2581}');
        let piece = text.trim_start_matches([' ', '\u{2581}']);
        if piece.is_empty() {
            continue;
        }
        match words.last_mut() {
            Some(w) if !boundary => {
                w.text.push_str(piece);
                w.end = end;
                w.conf = w.conf.zip(conf).map(|(a, b)| a.min(b));
            }
            _ => words.push(Word {
                text: piece.to_string(),
                start,
                end,
                conf,
            }),
        }
        boundary = false;
    }
    words
}

/// Revisions of one recording, kept in memory and written once at the end.
pub struct RevisionLog {
    started: Instant,
    rev: u32,
    lines: Vec<Value>,
}

impl Default for RevisionLog {
    fn default() -> Self {
        Self::new()
    }
}

impl RevisionLog {
    /// Start the clock: call when the first audio block arrives.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            rev: 0,
            lines: Vec::new(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Log a preview revision at the current wall time.
    pub fn preview(&mut self, audio_secs: f32, text: &str, words: Vec<Word>) {
        let t = self.now_ms();
        self.preview_at(t, audio_secs, text, words);
    }

    /// Log a preview revision at a given time (for offline replay, where
    /// wall time is simulated).
    pub fn preview_at(&mut self, t_ms: u64, audio_secs: f32, text: &str, words: Vec<Word>) {
        self.rev += 1;
        self.lines.push(json!({
            "kind": "preview",
            "rev": self.rev,
            "t_ms": t_ms,
            "audio_ms": (audio_secs * 1000.0).round() as u64,
            "text": text,
            "words": words,
        }));
    }

    /// Number of preview revisions logged so far.
    pub fn revisions(&self) -> u32 {
        self.rev
    }

    /// Write the log with its header and final line to
    /// `<dir>/<YYYY-MM>/<stem>.revisions.jsonl`. `final_text` is None when
    /// nothing was pasted. The directory is created with mode 0700, since
    /// the text is private speech.
    #[allow(clippy::too_many_arguments)] // one call site; a params struct would only move the list
    pub fn write(
        self,
        dir: &Path,
        stem: &str,
        header: Value,
        final_t_ms: Option<u64>,
        final_source: &str,
        final_model: &str,
        final_text: Option<&str>,
    ) -> io::Result<PathBuf> {
        let month = stem
            .get(..6)
            .map(|ym| format!("{}-{}", &ym[..4], &ym[4..]))
            .unwrap_or_default();
        let dir = dir.join(month);
        create_private_dir(&dir)?;
        let path = dir.join(format!("{}.revisions.jsonl", stem));
        let mut header = header;
        header["kind"] = json!("header");
        header["version"] = json!(REVISIONS_VERSION);
        let t_final = final_t_ms.unwrap_or_else(|| self.now_ms());
        let mut out = io::BufWriter::new(std::fs::File::create(&path)?);
        writeln!(out, "{}", header)?;
        for line in &self.lines {
            writeln!(out, "{}", line)?;
        }
        let last = json!({
            "kind": "final",
            "t_ms": t_final,
            "source": final_source,
            "model": final_model,
            "text": final_text,
        });
        writeln!(out, "{}", serde_json::to_string(&last)?)?;
        out.flush()?;
        Ok(path)
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Expand a leading `~/` to `$HOME`.
pub fn expand_home(dir: &str) -> PathBuf {
    match (dir.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_from_tokens_joins_subwords_and_keeps_repeats() {
        let tokens = [
            (" the", 0.0, 0.08),
            (" the", 0.08, 0.16),
            (" con", 0.2, 0.28),
            ("fig", 0.28, 0.36),
            (".", 0.36, 0.44),
        ];
        let words = words_from_tokens(tokens.iter().map(|&(t, s, e)| (t, s, e, None)));
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["the", "the", "config."]);
        assert_eq!(words[2].start, 0.2);
        assert_eq!(words[2].end, 0.44);
    }

    #[test]
    fn words_from_tokens_lone_marker_starts_next_word() {
        let tokens = [
            (" new", 0.0, 0.08),
            (" ", 0.08, 0.16),
            ("X", 0.16, 0.24),
            ("Y", 0.24, 0.32),
        ];
        let words = words_from_tokens(tokens.iter().map(|&(t, s, e)| (t, s, e, None)));
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["new", "XY"]);
        assert_eq!(words[1].start, 0.16);
    }

    #[test]
    fn word_confidence_is_lowest_token_score() {
        let words = words_from_tokens([
            (" con", 0.0, 0.1, Some(0.9)),
            ("fig", 0.1, 0.2, Some(0.4)),
            (" file", 0.2, 0.3, Some(0.8)),
        ]);
        assert_eq!(words[0].conf, Some(0.4));
        assert_eq!(words[1].conf, Some(0.8));
    }

    #[test]
    fn write_produces_header_revisions_and_final() {
        let dir = std::env::temp_dir().join(format!("voxtype-rev-test-{}", std::process::id()));
        let mut log = RevisionLog::new();
        log.preview_at(
            100,
            0.1,
            "hel",
            words_from_tokens([(" hel", 0.0, 0.1, None)]),
        );
        log.preview_at(
            200,
            0.2,
            "hello",
            words_from_tokens([(" hel", 0.0, 0.1, None), ("lo", 0.1, 0.2, None)]),
        );
        let path = log
            .write(
                &dir,
                "20261026-120000",
                json!({"source": "test"}),
                Some(300),
                "preview",
                "m",
                Some("Hello."),
            )
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0]["kind"], "header");
        assert_eq!(lines[0]["version"], REVISIONS_VERSION);
        assert_eq!(lines[2]["rev"], 2);
        assert_eq!(lines[2]["words"][0]["text"], "hello");
        assert_eq!(lines[3]["text"], "Hello.");
        assert!(path.parent().unwrap().ends_with("2026-10"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
