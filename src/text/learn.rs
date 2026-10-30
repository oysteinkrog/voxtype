//! Learning from spoken corrections.
//!
//! Each correction that changes the pasted text is logged as one JSON line
//! in the corrections log ([`TextConfig::corrections_log`]). A pair of
//! wrong and right words becomes a learned rule in the learned words file
//! ([`TextConfig::learned_words_file`], `"from" = "to"` lines like
//! `replacements_file`) the second time it is logged, or the first time
//! when the right words look like a name or a made-up word (an inner
//! capital, a digit, a letter outside ASCII, or all capitals). A learned
//! rule applies from the next text on; `text.replacements` and
//! `replacements_file` win over it.
//!
//! Learned words stay on this machine: they only replace words in the
//! output text and are never sent to an engine or a post-processing
//! command as a word list.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::{Config, TextConfig};

/// One applied correction in the corrections log. No audio.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// RFC 3339 local time.
    pub time: String,
    /// The live words replaced.
    pub wrong: String,
    /// The words that replaced them in the pasted text.
    pub right: String,
    /// The live text before any correction.
    pub live: String,
    /// The final engine's text before the corrections.
    #[serde(rename = "final")]
    pub final_text: String,
    /// The text with the corrections made, before text processing.
    pub pasted: String,
    /// The hold, in seconds into the recording.
    pub t0: f32,
    pub t1: f32,
}

/// Where corrections are logged and learned, from the config.
#[derive(Clone, Debug, PartialEq)]
pub struct Learning {
    pub log: PathBuf,
    pub rules: PathBuf,
    /// False: nothing is logged or learned.
    pub enabled: bool,
}

static ACTIVE: Mutex<Option<Learning>> = Mutex::new(None);

/// Sets the learning the daemon uses, so a placed correction can say in its
/// note that it will be learned.
pub fn set_active(learning: Option<Learning>) {
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = learning;
}

/// The learning the daemon set; None outside the daemon.
pub fn active() -> Option<Learning> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// A rule's key: the words in lower case without their punctuation, as
/// `text.replacements` matches them.
pub fn key(words: &str) -> String {
    words
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '-')
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The right words as a rule writes them: without sentence marks at the ends.
fn tidy(right: &str) -> String {
    right
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| ".,!?;:\"".contains(c))
        .to_string()
}

/// Words that a speech model would not write by itself: an inner capital
/// (FrankenTerm), a digit, a letter outside ASCII, or all capitals (NAS).
fn looks_like_a_name(right: &str) -> bool {
    right.split_whitespace().any(|w| {
        let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
        let inner_capital = letters.iter().skip(1).any(|c| c.is_uppercase());
        inner_capital
            || w.chars().any(|c| c.is_ascii_digit())
            || letters.iter().any(|c| !c.is_ascii())
    })
}

/// The rule a correction of `wrong` by `right` would make; None when it
/// makes none: the words are the same, or a wrong word is also among the
/// right ones (an insertion, "test" to "quiet test", which a rule would
/// make of every "test").
fn rule(wrong: &str, right: &str) -> Option<(String, String)> {
    let from = key(wrong);
    let to = tidy(right);
    let to_key = key(&to);
    if from.is_empty() || to.is_empty() || tidy(wrong) == to {
        return None;
    }
    let to_words: Vec<&str> = to_key.split(' ').collect();
    if from.split(' ').any(|w| to_words.contains(&w)) && from != to_key {
        return None;
    }
    Some((from, to))
}

impl Learning {
    pub fn from_config(text: &TextConfig) -> Self {
        let path = |set: &Option<String>, name: &str| {
            set.as_deref()
                .map(expand)
                .unwrap_or_else(|| Config::data_dir().join(name))
        };
        Self {
            log: path(&text.corrections_log, "corrections.jsonl"),
            rules: path(&text.learned_words_file, "words-learned.toml"),
            enabled: text.learn_corrections,
        }
    }

    /// The logged corrections; none when there is no log. Lines that do not
    /// parse are skipped.
    pub fn entries(&self) -> Vec<Entry> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn seen(&self, from: &str, to: &str) -> usize {
        self.entries()
            .iter()
            .filter(|e| rule(&e.wrong, &e.right).is_some_and(|(f, t)| f == from && t == to))
            .count()
    }

    /// Whether logging this correction once more learns it.
    pub fn would_learn(&self, wrong: &str, right: &str) -> bool {
        self.enabled
            && rule(wrong, right)
                .is_some_and(|(from, to)| looks_like_a_name(&to) || self.seen(&from, &to) >= 1)
    }

    /// Logs `entry` and learns its pair when it qualifies. Returns the new
    /// rule, if one was written.
    pub fn record(&self, entry: &Entry) -> std::io::Result<Option<(String, String)>> {
        if !self.enabled {
            return Ok(None);
        }
        let learn = self.would_learn(&entry.wrong, &entry.right);
        if let Some(dir) = self.log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)?;
        writeln!(log, "{}", serde_json::to_string(entry)?)?;
        let Some((from, to)) = rule(&entry.wrong, &entry.right).filter(|_| learn) else {
            return Ok(None);
        };
        let mut rules = self.rules();
        if rules.get(&from) == Some(&to) {
            return Ok(None);
        }
        rules.insert(from.clone(), to.clone());
        self.write_rules(&rules)?;
        tracing::info!(%from, %to, "Learned a word from a spoken correction");
        Ok(Some((from, to)))
    }

    /// The learned rules; none when there is no file or it does not parse.
    pub fn rules(&self) -> BTreeMap<String, String> {
        read_rules(&self.rules)
    }

    /// Removes the learned rule for `wrong`. Returns whether there was one.
    pub fn forget(&self, wrong: &str) -> std::io::Result<bool> {
        let mut rules = self.rules();
        if rules.remove(&key(wrong)).is_none() {
            return Ok(false);
        }
        self.write_rules(&rules)?;
        Ok(true)
    }

    fn write_rules(&self, rules: &BTreeMap<String, String>) -> std::io::Result<()> {
        if let Some(dir) = self.rules.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string(rules).map_err(std::io::Error::other)?;
        let text = format!(
            "# Words voxtype learned from spoken corrections. Same form as\n\
             # text.replacements_file; hand rules win. Remove one with\n\
             # `voxtype glossary forget <words>`.\n{body}"
        );
        let tmp = self.rules.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.rules)
    }
}

fn read_rules(path: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    match toml::from_str(&text) {
        Ok(rules) => rules,
        Err(e) => {
            tracing::warn!("Skipping learned words {:?}: {}", path, e);
            BTreeMap::new()
        }
    }
}

/// The current local time for [`Entry::time`].
pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learning(dir: &Path) -> Learning {
        Learning {
            log: dir.join("log/corrections.jsonl"),
            rules: dir.join("words-learned.toml"),
            enabled: true,
        }
    }

    fn entry(wrong: &str, right: &str) -> Entry {
        Entry {
            wrong: wrong.into(),
            right: right.into(),
            ..Entry::default()
        }
    }

    #[test]
    fn a_pair_seen_twice_writes_the_rule() {
        let dir = tempfile::tempdir().unwrap();
        let l = learning(dir.path());
        assert!(!l.would_learn("blue barrel", "blueberry"));
        assert_eq!(l.record(&entry("blue barrel", "blueberry")).unwrap(), None);
        assert!(l.rules().is_empty());
        assert!(l.would_learn("Blue barrel,", "blueberry."));
        assert_eq!(
            l.record(&entry("Blue barrel,", "blueberry.")).unwrap(),
            Some(("blue barrel".into(), "blueberry".into()))
        );
        assert_eq!(
            l.rules().get("blue barrel").map(String::as_str),
            Some("blueberry")
        );
        assert_eq!(l.entries().len(), 2);
    }

    #[test]
    fn a_name_is_learned_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let l = learning(dir.path());
        for (wrong, right) in [
            ("glim bot", "GlimBot"),
            ("zed nine", "Zed9"),
            ("smorbrod", "Smørbrød"),
            ("pdq", "PDQ"),
        ] {
            assert!(l.would_learn(wrong, right), "{right}");
            assert!(l.record(&entry(wrong, right)).unwrap().is_some(), "{right}");
        }
        assert_eq!(l.rules().len(), 4);
    }

    #[test]
    fn an_insertion_or_a_repeat_is_not_learned() {
        let dir = tempfile::tempdir().unwrap();
        let l = learning(dir.path());
        for _ in 0..2 {
            l.record(&entry("test", "quiet test")).unwrap();
            l.record(&entry("GlimBot", "GlimBot")).unwrap();
        }
        assert!(l.rules().is_empty());
        // A change of case is a rule.
        assert!(l.would_learn("pdq", "PDQ"));
    }

    #[test]
    fn forget_removes_the_rule() {
        let dir = tempfile::tempdir().unwrap();
        let l = learning(dir.path());
        l.record(&entry("glim bot", "GlimBot")).unwrap();
        assert!(l.forget("Glim Bot").unwrap());
        assert!(l.rules().is_empty());
        assert!(!l.forget("glim bot").unwrap());
    }

    #[test]
    fn nothing_is_logged_when_learning_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let l = Learning {
            enabled: false,
            ..learning(dir.path())
        };
        assert!(!l.would_learn("glim bot", "GlimBot"));
        assert_eq!(l.record(&entry("glim bot", "GlimBot")).unwrap(), None);
        assert!(!l.log.exists());
        assert!(!l.rules.exists());
    }

    #[test]
    fn the_paths_come_from_the_config() {
        let text = TextConfig {
            corrections_log: Some("/var/lib/x/log.jsonl".into()),
            learned_words_file: Some("/var/lib/x/words.toml".into()),
            ..TextConfig::default()
        };
        let l = Learning::from_config(&text);
        assert_eq!(l.log, PathBuf::from("/var/lib/x/log.jsonl"));
        assert_eq!(l.rules, PathBuf::from("/var/lib/x/words.toml"));
        let l = Learning::from_config(&TextConfig::default());
        assert!(l.enabled);
        assert_eq!(l.log, Config::data_dir().join("corrections.jsonl"));
        assert_eq!(l.rules, Config::data_dir().join("words-learned.toml"));
    }
}
