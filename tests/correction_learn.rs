//! A spoken correction made twice becomes a learned word (COR-6).
//!
//! Replays the live words of tests/fixtures/correction/speech_long.live.json
//! with one word made wrong ("activity" heard as "captivity") and a hold in
//! which the right word is respoken, as tests/correction_replay.rs does, and
//! then logs the pasted corrections as the daemon does. The first time is
//! logged only; the second time the popup note says it is learned and the
//! rule is written. The log and the rules go to a scratch directory.

use std::path::PathBuf;
use voxtype::config::TextConfig;
use voxtype::osd::correction;
use voxtype::osd::live::{self, TimedWord, WordLine};
use voxtype::text::learn::{self, Learning};
use voxtype::text::TextProcessor;

const WRONG: &str = "captivity";
const RIGHT: &str = "activity";
const T0: f32 = 5.0;
const T1: f32 = 5.9;

fn steps() -> Vec<Vec<WordLine>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/correction/speech_long.live.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn word(text: &str, start: f32, end: f32) -> WordLine {
    WordLine {
        word: TimedWord {
            text: text.into(),
            start,
            end,
            conf: Some(0.9),
        },
        detail: None,
    }
}

fn live_step(words: Vec<WordLine>) -> String {
    let text = words
        .iter()
        .map(|w| w.word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    live::word_lines(words);
    live::preview(&text)
}

/// One dictation with the misheard word and the hold; returns the pasted
/// text and the popup's note.
fn dictate(learning: &Learning) -> (String, correction::CorrectionNote) {
    let mut steps = steps();
    for step in &mut steps {
        for w in step.iter_mut() {
            if w.word.text == RIGHT {
                w.word.text = WRONG.into();
            }
        }
    }
    let mut held = steps.last().unwrap().clone();
    for step in steps {
        live_step(step);
    }
    correction::start(T0);
    live::refresh_correction();
    held.push(word(RIGHT, 5.3, 5.6));
    live_step(held.clone());
    correction::end(T1);
    live::refresh_correction();
    held.push(word("done", 6.2, 6.4));
    live_step(held);
    let note = correction::view()
        .and_then(|v| v.note)
        .expect("a correction note");
    let pasted = correction::apply_final(
        "This is a longer test of voice captivity detection \
         with multiple words and phrases. Activity. Done.",
    );
    for entry in correction::applied() {
        learning.record(&entry).unwrap();
    }
    live::clear();
    (pasted, note)
}

#[test]
fn a_correction_made_twice_is_learned_and_applied() {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("correction_learn");
    for old in ["corrections.jsonl", "words-learned.toml"] {
        let _ = std::fs::remove_file(scratch.join("words").join(old));
    }
    std::fs::create_dir_all(scratch.join("voxtype")).unwrap();
    let no_tools = scratch.join("no-tools");
    std::fs::create_dir_all(&no_tools).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &scratch);
    std::env::set_var("PATH", &no_tools);
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    std::env::remove_var("SWAYSOCK");

    let text = TextConfig {
        corrections_log: Some(
            scratch
                .join("words/corrections.jsonl")
                .display()
                .to_string(),
        ),
        learned_words_file: Some(
            scratch
                .join("words/words-learned.toml")
                .display()
                .to_string(),
        ),
        ..TextConfig::default()
    };
    let learning = Learning::from_config(&text);
    learn::set_active(Some(learning.clone()));
    let fixed = "This is a longer test of voice activity detection \
                 with multiple words and phrases. Done.";

    let (pasted, note) = dictate(&learning);
    assert_eq!(pasted, fixed);
    assert_eq!((note.from.as_str(), note.to.as_str()), (WRONG, RIGHT));
    assert!(!note.learned, "once is not enough for an ordinary word");
    assert!(learning.rules().is_empty());
    assert_eq!(learning.entries().len(), 1);

    let (pasted, note) = dictate(&learning);
    assert_eq!(pasted, fixed);
    assert!(note.learned, "the popup says it is learned");
    assert_eq!(learning.rules().get(WRONG).map(String::as_str), Some(RIGHT));
    let entry = &learning.entries()[1];
    assert_eq!((entry.wrong.as_str(), entry.right.as_str()), (WRONG, RIGHT));
    assert_eq!(entry.pasted, fixed);

    // The next text gets the rule without a correction.
    let mut processor = TextProcessor::new(&text);
    processor.set_learned(learning.rules());
    assert_eq!(
        processor.process("voice captivity detection"),
        "voice activity detection"
    );
    learn::set_active(None);
}
