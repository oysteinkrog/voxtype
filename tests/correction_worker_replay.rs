//! Replays a real live worker run with a spoken correction through the
//! daemon's side: the worker's word lists and preview texts as they came,
//! the correction key at the hold's times, then the worker's answer.
//!
//! The run is a JSON file from a driver that fed a wav to
//! `voxtype transcribe-worker --live-model` in real time while
//! `correction-windows.json` listed the hold: `{"clips": [{"start_t": s,
//! "lines": [[arrival_s, line], ...]}]}`. Ignored, as it needs such a run:
//!
//! ```text
//! VOXTYPE_REPLAY=run.json VOXTYPE_REPLAY_HOLD=5.05,6.41 \
//!     cargo test --test correction_worker_replay -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use voxtype::osd::correction::{self, FinalClip};
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

#[test]
#[ignore]
fn a_recorded_worker_run_with_a_hold_gives_the_pasted_text() {
    let run = std::env::var("VOXTYPE_REPLAY").expect("VOXTYPE_REPLAY: a driver run");
    let hold = std::env::var("VOXTYPE_REPLAY_HOLD").expect("VOXTYPE_REPLAY_HOLD: t0,t1");
    let (t0, t1) = hold.split_once(',').expect("t0,t1");
    let (t0, t1): (f32, f32) = (t0.parse().unwrap(), t1.parse().unwrap());

    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("correction_worker_replay");
    std::fs::create_dir_all(scratch.join("voxtype")).unwrap();
    let no_tools = scratch.join("no-tools");
    std::fs::create_dir_all(&no_tools).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &scratch);
    std::env::set_var("PATH", &no_tools);
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    std::env::remove_var("SWAYSOCK");

    let run: Run = serde_json::from_str(&std::fs::read_to_string(run).unwrap()).unwrap();
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
    if let Some(note) = correction::view().and_then(|v| v.note) {
        eprintln!("note {note:?}");
    }
    eprintln!("pasted {pasted:?}");
    live::clear();
}
