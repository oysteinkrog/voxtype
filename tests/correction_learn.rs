//! A spoken correction made twice becomes a learned word (COR-6).
//!
//! Replays the live words of tests/fixtures/correction/speech_long.live.json
//! with one word made wrong ("activity" heard as "captivity") and a hold in
//! which the right word is respoken, as tests/correction_replay.rs does, and
//! then logs the pasted corrections as the daemon does. The first time is
//! logged only; the second time the popup note says it is learned and the
//! rule is written. The log and the rules go to a scratch directory.

use std::path::PathBuf;
use std::sync::Mutex;
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

/// One dictation with the misheard word and a hold of `said` (words a
/// third of a second apart) ending at `t1`, then "done". Returns the
/// pasted text of `final_text` and the popup's note.
fn dictate_hold(
    learning: &Learning,
    said: &[&str],
    t1: f32,
    final_text: &str,
) -> (String, correction::CorrectionNote) {
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
    for (i, w) in said.iter().enumerate() {
        let start = T0 + 0.2 + 0.33 * i as f32;
        held.push(word(w, start, start + 0.25));
        live_step(held.clone());
    }
    correction::end(t1);
    live::refresh_correction();
    held.push(word("done", t1 + 0.3, t1 + 0.5));
    live_step(held);
    let note = correction::view()
        .and_then(|v| v.note)
        .expect("a correction note");
    let pasted = correction::apply_final(final_text);
    for entry in correction::applied() {
        learning.record(&entry).unwrap();
    }
    live::clear();
    (pasted, note)
}

fn dictate(learning: &Learning) -> (String, correction::CorrectionNote) {
    dictate_hold(
        learning,
        &[RIGHT],
        T1,
        "This is a longer test of voice captivity detection \
         with multiple words and phrases. Activity. Done.",
    )
}

static TURN: Mutex<()> = Mutex::new(());

/// Points the runtime folder at a scratch folder and returns the text
/// config with the log and the rules in `<scratch>/<name>/`, emptied.
fn scratch(name: &str) -> TextConfig {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("correction_learn");
    for old in ["corrections.jsonl", "words-learned.toml"] {
        let _ = std::fs::remove_file(scratch.join(name).join(old));
    }
    std::fs::create_dir_all(scratch.join("voxtype")).unwrap();
    let no_tools = scratch.join("no-tools");
    std::fs::create_dir_all(&no_tools).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &scratch);
    std::env::set_var("PATH", &no_tools);
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    std::env::remove_var("SWAYSOCK");

    let path = |file: &str| Some(scratch.join(name).join(file).display().to_string());
    TextConfig {
        corrections_log: path("corrections.jsonl"),
        learned_words_file: path("words-learned.toml"),
        ..TextConfig::default()
    }
}

#[test]
fn a_correction_made_twice_is_learned_and_applied() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let text = scratch("twice");
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

/// A spelled word is placed and learned the first time (COR-5).
#[test]
fn a_spelled_word_is_learned_at_once() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let text = scratch("spelled");
    let learning = Learning::from_config(&text);
    learn::set_active(Some(learning.clone()));
    let said = [
        "Spell", "alpha", "charlie", "tango", "india", "victor", "india", "tango", "yankee.",
    ];
    // live_preview_final = "preview": the final text is the live words.
    let (pasted, note) = dictate_hold(
        &learning,
        &said,
        8.2,
        "This is a longer test of voice captivity detection with multiple words \
         and phrases Spell alpha charlie tango india victor india tango yankee. done",
    );
    assert_eq!(
        pasted,
        "This is a longer test of voice activity detection with multiple words \
         and phrases done"
    );
    assert_eq!((note.from.as_str(), note.to.as_str()), (WRONG, RIGHT));
    assert!(note.learned);
    assert_eq!(learning.rules().get(WRONG).map(String::as_str), Some(RIGHT));
    assert!(learning.entries()[0].spelled);
    learn::set_active(None);
}
