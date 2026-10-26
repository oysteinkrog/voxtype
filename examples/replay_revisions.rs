//! Replay kept recordings through the live preview model and write a
//! revision log for each, as the live worker would with
//! `live_preview_revisions_dir` set.
//!
//! Input is a `live_preview_corpus_dir`: `<time>.wav` (16 kHz mono) plus
//! `<time>.json` with the final text that was pasted. The final line of
//! each log is that recorded final text, so the final swap is the real one.
//!
//! Audio is fed in 100 ms blocks. Wall time is simulated: a block arrives
//! when its audio would have been spoken, and its revision is out once the
//! model has processed it (measured here), so `t_ms` includes model lag.
//!
//! ```sh
//! cargo run --example replay_revisions --features parakeet -- \
//!     ~/.local/share/voxtype/corpus OUT_DIR [MODEL_DIR]
//! ```

use parakeet_rs::{ParakeetUnified, ParakeetUnifiedHandle, TimestampMode, UnifiedStreamingConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;
use voxtype::transcribe::revisions::{expand_home, words_from_tokens, RevisionLog, Word};

const BLOCK: usize = 1600;

fn words(u: &ParakeetUnified) -> Vec<Word> {
    let timed = u.get_timed_transcript(TimestampMode::Tokens);
    let conf = u.token_confidences();
    words_from_tokens(
        timed
            .tokens
            .iter()
            .enumerate()
            .map(|(i, t)| (t.text.as_str(), t.start, t.end, conf.get(i).copied())),
    )
}

fn wavs(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            wavs(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "wav") {
            out.push(path);
        }
    }
    Ok(())
}

fn read_wav(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    anyhow::ensure!(spec.sample_rate == 16000 && spec.channels == 1, "need 16 kHz mono");
    Ok(reader
        .samples::<i16>()
        .map(|s| s.map(|x| x as f32 / i16::MAX as f32))
        .collect::<Result<_, _>>()?)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(args.len() >= 3, "usage: replay_revisions CORPUS_DIR OUT_DIR [MODEL_DIR]");
    let corpus = expand_home(&args[1]);
    let out_dir = expand_home(&args[2]);
    let model = args
        .get(3)
        .map(|m| expand_home(m))
        .unwrap_or_else(|| expand_home("~/.local/share/voxtype/models/parakeet-unified-en-0.6b-int8"));
    let model_name = model.file_name().unwrap_or_default().to_string_lossy().to_string();

    let handle = ParakeetUnifiedHandle::load(&model, None)?;
    let mut files = Vec::new();
    wavs(&corpus, &mut files)?;
    files.sort();

    let (mut n, mut same_preview, mut audio_total, mut revs_total) = (0, 0, 0.0f32, 0u32);
    for wav in &files {
        let meta: Value = match std::fs::read_to_string(wav.with_extension("json")) {
            Ok(s) => serde_json::from_str(&s)?,
            Err(_) => continue,
        };
        let samples = read_wav(wav)?;
        let mut u = ParakeetUnified::from_shared_with_streaming_config(
            &handle,
            UnifiedStreamingConfig::default(),
        )?;
        let mut log = RevisionLog::new();
        let mut text = String::new();
        // Simulated wall clock: when the model is free again, in ms.
        let mut busy_until = 0.0f64;
        let mut fed = 0usize;
        for block in samples.chunks(BLOCK) {
            fed += block.len();
            let arrive = fed as f64 / 16.0;
            let start = Instant::now();
            let delta = u.transcribe_chunk(block)?;
            busy_until = busy_until.max(arrive) + start.elapsed().as_secs_f64() * 1000.0;
            if !delta.is_empty() {
                text.push_str(&delta);
                log.preview_at(busy_until as u64, fed as f32 / 16000.0, text.trim_start(), words(&u));
            }
        }
        let start = Instant::now();
        let tail = u.flush()?;
        busy_until += start.elapsed().as_secs_f64() * 1000.0;
        if !tail.is_empty() {
            text.push_str(&tail);
            log.preview_at(busy_until as u64, fed as f32 / 16000.0, text.trim(), words(&u));
        }
        let preview = text.trim();
        if meta["preview_text"].as_str() == Some(preview) {
            same_preview += 1;
        }
        let final_text = meta["final_text"].as_str();
        let final_model = meta["final_model"].as_str().unwrap_or_default();
        let source = if final_model == "preview" { "preview" } else { "engine" };
        let stem = wav.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let header = json!({
            "source": "replay",
            "time": meta["time"],
            "wav": wav.file_name().map(|f| f.to_string_lossy().to_string()),
            "seconds": samples.len() as f32 / 16000.0,
            "preview_model": model_name,
            "recorded_preview_model": meta["preview_model"],
            "final_model": final_model,
            "block_samples": BLOCK,
        });
        n += 1;
        audio_total += samples.len() as f32 / 16000.0;
        revs_total += log.revisions();
        let revs = log.revisions();
        let final_ms = Some(busy_until as u64);
        let path = log.write(&out_dir, &stem, header, final_ms, source, final_model, final_text)?;
        eprintln!("{} revisions -> {}", revs, path.display());
    }
    eprintln!(
        "{} recordings, {:.0} s of audio, {} revisions, replayed preview equals the recorded one in {}",
        n, audio_total, revs_total, same_preview
    );
    Ok(())
}
