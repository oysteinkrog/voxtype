//! Replays a real live worker run with a spoken correction through the
//! daemon's side: the worker's word lists and preview texts as they came,
//! the correction key at the hold's times, then the worker's answer.
//!
//! A run is a JSON file from a driver that fed a wav to
//! `voxtype transcribe-worker --live-model` in real time while
//! `correction-windows.json` listed the hold: `{"clips": [{"start_t": s,
//! "lines": [[arrival_s, line], ...]}]}`.
//!
//! tests/fixtures/correction/long_quiet_hold.worker.json is such a run:
//! the public fixtures speech_long.wav, 0.6 s of silence and
//! speech_quiet.wav, with the key held over "Quiet test." (5.05 to 6.41 s),
//! and Cohere writing the final text. long_quiet_hold.preview.json is the
//! same with `live_preview_final = "preview"`: the final text is the live
//! words, the respoken ones included.
//! Another run replays with the ignored test:
//!
//! ```text
//! VOXTYPE_REPLAY=run.json VOXTYPE_REPLAY_HOLD=5.05,6.41 \
//!     cargo test --test correction_worker_replay -- --ignored --nocapture
//! ```
//!
//! The live state is process-wide and writes to the runtime directory, so
//! the tests take turns.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use voxtype::osd::correction::{self, CorrectionNote, FinalClip};
use voxtype::osd::live::{self, WordLine};

#[derive(serde::Deserialize)]
struct Run {
    clips: Vec<Clip>,
}

#[derive(serde::Deserialize)]
struct Clip {
    start_t: f32,
    lines: Vec<(f32, String)>,
}

#[derive(serde::Deserialize)]
struct Answer {
    text: Option<String>,
    #[serde(default)]
    corrections: Vec<FinalClip>,
}

static TURN: Mutex<()> = Mutex::new(());

/// The pasted text of the run at `path` with the key held `t0..t1`, and
/// the note of the newest correction.
fn replay(path: &Path, t0: f32, t1: f32) -> (String, Option<CorrectionNote>) {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("correction_worker_replay");
    std::fs::create_dir_all(scratch.join("voxtype")).unwrap();
    let no_tools = scratch.join("no-tools");
    std::fs::create_dir_all(&no_tools).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &scratch);
    std::env::set_var("PATH", &no_tools);
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    std::env::remove_var("SWAYSOCK");

    let run: Run = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let clip = &run.clips[0];
    let (mut started, mut ended) = (false, false);
    let mut answer = None;
    for (at, line) in &clip.lines {
        // The worker reads the audio in real time: arrival time is audio time.
        let at = at - clip.start_t;
        if !started && at >= t0 {
            correction::start(t0);
            live::refresh_correction();
            started = true;
        }
        if !ended && at >= t1 {
            correction::end(t1);
            live::refresh_correction();
            ended = true;
        }
        if let Some(json) = line.strip_prefix("W ") {
            live::word_lines(serde_json::from_str::<Vec<WordLine>>(json).unwrap());
        } else if let Some(json) = line.strip_prefix("P ") {
            let text: String = serde_json::from_str(json).unwrap();
            let shown = live::preview(&text);
            eprintln!("{at:5.2} live {shown:?}");
        } else if line.starts_with('{') {
            answer = Some(serde_json::from_str::<Answer>(line).unwrap());
        }
    }
    if !ended {
        correction::end(t1);
    }
    let answer = answer.expect("the worker's answer");
    let final_text = answer.text.unwrap_or_default();
    eprintln!("final {final_text:?}");
    eprintln!("clips {:?}", answer.corrections);
    correction::final_clips(answer.corrections);
    let pasted = correction::apply_final(&final_text);
    let note = correction::view().and_then(|v| v.note);
    eprintln!("note {note:?}");
    eprintln!("pasted {pasted:?}");
    live::clear();
    (pasted, note)
}

/// Pushing "test" down made Cohere write the hold as "Client's Pest", which
/// fits nowhere in the final text: the live words "Quiet test." go in.
#[test]
fn a_clip_that_fits_nowhere_pastes_the_live_words() {
    let run = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/correction/long_quiet_hold.worker.json");
    let (pasted, note) = replay(&run, 5.05, 6.41);
    assert_eq!(
        pasted,
        "This is a longer quiet test of voice activity detection with multiple words and phrases."
    );
    let note = note.expect("a note");
    assert_eq!(note.dropped, None);
    assert_eq!(note.from, "test");
}

/// With the live words as the final text, the respoken copy goes and the
/// correction goes in.
#[test]
fn a_preview_final_text_gets_the_correction() {
    let run = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/correction/long_quiet_hold.preview.json");
    let (pasted, note) = replay(&run, 5.05, 6.41);
    assert_eq!(
        pasted,
        "This is a longer quiet test of voice activity detection with multiple words and phrases"
    );
    assert_eq!(note.expect("a note").dropped, None);
}

#[test]
#[ignore]
fn a_recorded_worker_run_with_a_hold_gives_the_pasted_text() {
    let run = std::env::var("VOXTYPE_REPLAY").expect("VOXTYPE_REPLAY: a driver run");
    let hold = std::env::var("VOXTYPE_REPLAY_HOLD").expect("VOXTYPE_REPLAY_HOLD: t0,t1");
    let (t0, t1) = hold.split_once(',').expect("t0,t1");
    replay(Path::new(&run), t0.parse().unwrap(), t1.parse().unwrap());
}
