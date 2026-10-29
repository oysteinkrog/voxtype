//! Replays a spoken correction through the live preview code path.
//!
//! The live words come from tests/fixtures/correction/speech_long.live.json:
//! the live model's word lists, step by step, for the public fixture
//! tests/fixtures/vad/speech_long.wav (recorded by the ignored test
//! `record_live_words_of_a_public_fixture` in src/transcribe/worker.rs). The
//! test makes one word wrong, holds a fake correction window after the
//! sentence in which the right word is spoken, and calls the functions the
//! worker reader and the daemon call, in their order: `live::word_lines`
//! and `live::preview` for each step, `correction::start` and `end` with
//! `live::refresh_correction` for the key, `correction::final_clips` for the
//! worker's answer, and `correction::apply_final` for the pasted text.
//!
//! The live state is process-wide and writes to the runtime directory, so
//! this file holds one test, which points XDG_RUNTIME_DIR at a scratch
//! directory before anything else.

use std::path::PathBuf;
use voxtype::osd::correction::{self, FinalClip};
use voxtype::osd::live::{self, TimedWord, WordLine};

const WRONG: &str = "captivity";
const RIGHT: &str = "activity";
/// The fake hold: the key goes down after the sentence and comes up after
/// the respoken word.
const T0: f32 = 5.0;
const T1: f32 = 5.9;

fn steps() -> Vec<Vec<WordLine>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/correction/speech_long.live.json");
    let json = std::fs::read_to_string(&path).expect("fixture reads");
    serde_json::from_str(&json).expect("fixture parses")
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

/// What the worker reader and the daemon do with one live step: the word
/// list, then the preview text. Returns the text the popup is shown.
fn live_step(words: Vec<WordLine>) -> String {
    let text = words
        .iter()
        .map(|w| w.word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    live::word_lines(words);
    live::preview(&text)
}

/// One dictation of the fixture with the misheard word and the hold.
/// Returns the live text shown at the end.
fn dictate() -> String {
    let mut steps = steps();
    for step in &mut steps {
        for w in step.iter_mut() {
            if w.word.text == RIGHT {
                w.word.text = WRONG.into();
            }
        }
    }
    let sentence = steps.last().unwrap().clone();
    let mut shown = String::new();
    for step in steps {
        shown = live_step(step);
    }
    assert!(
        shown.contains(WRONG),
        "the live words have the mistake: {shown}"
    );

    correction::start(T0);
    live::refresh_correction();
    let windows = correction::read_windows();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].t1, None, "the worker sees an open hold");

    let mut held = sentence.clone();
    held.push(word(RIGHT, 5.3, 5.6));
    let shown = live_step(held.clone());
    assert!(
        !shown.ends_with(RIGHT),
        "the respoken word is not dictation: {shown}"
    );
    let view = correction::view().expect("a correction view");
    assert_eq!(view.hearing.as_deref(), Some(RIGHT));

    correction::end(T1);
    live::refresh_correction();
    assert_eq!(correction::read_windows()[0].t1, Some(T1));

    // The speaker goes on; the live model has heard past the hold.
    held.push(word("done", 6.2, 6.4));
    live_step(held)
}

#[test]
fn a_spoken_correction_fixes_the_live_text_and_the_pasted_text() {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("correction_replay");
    std::fs::create_dir_all(scratch.join("voxtype")).unwrap();
    let no_tools = scratch.join("no-tools");
    std::fs::create_dir_all(&no_tools).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &scratch);
    // The first preview looks up the focused window with kdotool, hyprctl
    // or swaymsg; none of them may reach the real desktop from a test.
    std::env::set_var("PATH", &no_tools);
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    std::env::remove_var("SWAYSOCK");

    let fixed = "This is a longer test of voice activity detection \
                 with multiple words and phrases.";

    // The final pass heard the hold too: its copy of the word goes.
    let shown = dictate();
    assert_eq!(shown, format!("{fixed} done"));
    let doc = live::read().expect("preview.json");
    assert_eq!(doc.text, shown);
    let note = doc.correction.and_then(|c| c.note).expect("a note");
    assert_eq!((note.from.as_str(), note.to.as_str()), (WRONG, RIGHT));
    assert_eq!(note.dropped, None);
    let pasted = correction::apply_final(
        "This is a longer test of voice captivity detection \
         with multiple words and phrases. Activity. Done.",
    );
    assert_eq!(pasted, format!("{fixed} Done."));
    live::clear();
    assert!(correction::read_windows().is_empty());

    // The worker cut the hold out of the final pass and decoded it alone.
    let shown = dictate();
    assert_eq!(shown, format!("{fixed} done"));
    correction::final_clips(vec![FinalClip {
        t0: T0,
        t1: T1,
        text: "Activity.".into(),
        cut: true,
    }]);
    let pasted = correction::apply_final(
        "This is a longer test of voice captivity detection \
         with multiple words and phrases. Done.",
    );
    assert_eq!(pasted, format!("{fixed} Done."));
    live::clear();
}
