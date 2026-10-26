//! Give replayed revision logs an engine final: transcribe each kept
//! recording with the configured engine (e.g. Cohere), in the same parts
//! the live worker sends it, and write a copy of each log with that text
//! as its final line.
//!
//! Input: revision logs from the `replay_revisions` example, and the
//! `live_preview_corpus_dir` holding the WAVs their headers name.
//!
//! ```sh
//! cargo run --example replay_engine_final --features cohere -- \
//!     REVISIONS_DIR CORPUS_DIR OUT_DIR
//! ```

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use voxtype::transcribe::revisions::expand_home;
use voxtype::transcribe::worker::speech_parts;

fn find(dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            find(&path, suffix, out)?;
        } else if path.to_string_lossy().ends_with(suffix) {
            out.push(path);
        }
    }
    Ok(())
}

fn read_wav(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    Ok(reader
        .samples::<i16>()
        .map(|s| s.map(|x| x as f32 / i16::MAX as f32))
        .collect::<Result<_, _>>()?)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(args.len() == 4, "usage: replay_engine_final REVISIONS_DIR CORPUS_DIR OUT_DIR");
    let (revs_dir, corpus, out_dir) = (expand_home(&args[1]), expand_home(&args[2]), expand_home(&args[3]));

    let mut config = voxtype::config::load_config(None)?;
    // Load the engine in this process, as the live worker's engine thread does.
    config.engine_isolation = false;
    let model = config.model_name().to_string();
    let start = Instant::now();
    let transcriber = voxtype::transcribe::create_transcriber(&config)?;
    eprintln!("{} loaded in {:.1} s", model, start.elapsed().as_secs_f32());

    let mut logs = Vec::new();
    find(&revs_dir, ".revisions.jsonl", &mut logs)?;
    logs.sort();
    let mut wavs = Vec::new();
    find(&corpus, ".wav", &mut wavs)?;

    let (mut n, mut audio, mut took) = (0, 0.0f32, 0.0f32);
    for log in &logs {
        let body = std::fs::read_to_string(log)?;
        let mut lines: Vec<Value> = body.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
        let Some(name) = lines.first().and_then(|h| h["wav"].as_str()).map(str::to_string) else {
            continue;
        };
        let Some(wav) = wavs.iter().find(|w| w.file_name().is_some_and(|f| f == name.as_str())) else {
            continue;
        };
        let samples = read_wav(wav)?;
        let t = Instant::now();
        let mut texts = Vec::new();
        for part in speech_parts(&samples) {
            let text = transcriber.transcribe(&samples[part])?;
            if !text.trim().is_empty() {
                texts.push(text.trim().to_string());
            }
        }
        took += t.elapsed().as_secs_f32();
        audio += samples.len() as f32 / 16000.0;
        let final_text = (!texts.is_empty()).then(|| texts.join(" "));
        let last_t = lines.iter().filter_map(|l| l["t_ms"].as_u64()).max().unwrap_or(0);
        lines.retain(|l| l["kind"] != "final");
        lines[0]["final_model"] = json!(model);
        lines.push(json!({
            "kind": "final",
            "t_ms": last_t,
            "source": "engine",
            "model": model,
            "text": final_text,
        }));
        let rel = log.strip_prefix(&revs_dir)?;
        let path = out_dir.join(rel);
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path.parent().unwrap())?;
        }
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for line in &lines {
            writeln!(out, "{}", line)?;
        }
        n += 1;
    }
    eprintln!("{} logs, {:.0} s of audio transcribed in {:.0} s", n, audio, took);
    Ok(())
}
