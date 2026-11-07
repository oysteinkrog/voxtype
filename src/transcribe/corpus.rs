//! The dictation corpus (`live_preview_corpus_dir`): each live preview
//! recording as `<dir>/<YYYY-MM>/<YYYYMMDD-HHMMSS.mmm>.wav` with a `.json`
//! sidecar of its texts.
//!
//! Finished recordings are saved by the live worker. A cancelled one is
//! saved by the daemon, which still holds the audio after the worker is
//! killed; its sidecar has `"outcome": "cancelled"` and no `final_text`.

use std::path::{Path, PathBuf};

/// `outcome` of a recording whose text went out.
pub const OUTCOME_DONE: &str = "done";
/// `outcome` of a recording cancelled before its text went out.
pub const OUTCOME_CANCELLED: &str = "cancelled";

/// What a cancelled live preview session leaves behind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CancelledTexts {
    /// The running transcript at the cancel.
    pub preview_text: String,
    /// The final model's text of each part it had already transcribed.
    pub final_parts: Vec<String>,
}

/// Write `samples` (16 kHz mono) and `meta` under `dir`, stamped with the
/// local time. `meta` gets `time` and `seconds` added. Returns the path
/// without extension.
pub fn save(dir: &Path, samples: &[f32], mut meta: serde_json::Value) -> anyhow::Result<PathBuf> {
    let now = chrono::Local::now();
    let month = dir.join(now.format("%Y-%m").to_string());
    let stem = month.join(now.format("%Y%m%d-%H%M%S%.3f").to_string());
    std::fs::create_dir_all(&month)?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create(stem.with_extension("wav"), spec)?;
    for &x in samples {
        wav.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    wav.finalize()?;
    if let Some(m) = meta.as_object_mut() {
        m.insert("time".into(), now.to_rfc3339().into());
        m.insert("seconds".into(), (samples.len() as f32 / 16000.0).into());
    }
    std::fs::write(
        stem.with_extension("json"),
        serde_json::to_string_pretty(&meta)?,
    )?;
    Ok(stem)
}

/// The sidecar of a cancelled recording: the same fields as a finished
/// one, with `final_text` null, plus `outcome` and `final_parts`.
pub fn cancelled_meta(config: &crate::config::Config, texts: &CancelledTexts) -> serde_json::Value {
    serde_json::json!({
        "outcome": OUTCOME_CANCELLED,
        "final_text": serde_json::Value::Null,
        "final_parts": texts.final_parts,
        "final_model": config.model_name(),
        "preview_text": texts.preview_text,
        "preview_model": config.live_preview_model,
        "corrected_text": serde_json::Value::Null,
    })
}

/// The disk work of saving a cancelled recording, to run off the cancel
/// path. `None` when the corpus is off or nothing was captured: a cancel
/// with no audio writes nothing.
pub fn save_cancelled(
    config: &crate::config::Config,
    samples: Vec<f32>,
    texts: &CancelledTexts,
) -> Option<impl FnOnce() + Send + 'static> {
    let dir = config.live_preview_corpus_dir.as_deref()?;
    if samples.is_empty() {
        return None;
    }
    let dir = super::revisions::expand_home(dir);
    let meta = cancelled_meta(config, texts);
    Some(move || match save(&dir, &samples, meta) {
        Ok(stem) => tracing::info!("Saved the cancelled recording to {}.wav", stem.display()),
        Err(e) => tracing::warn!("Could not save the cancelled recording: {}", e),
    })
}

/// For tests: the one recording under `dir`, as its WAV length in
/// samples and its sidecar.
#[cfg(test)]
pub(crate) fn only_recording(dir: &Path) -> Option<(u32, serde_json::Value)> {
    let month = std::fs::read_dir(dir).ok()?.next()?.ok()?.path();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&month)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    assert_eq!(files.len(), 2, "{files:?}");
    let wav = hound::WavReader::open(&files[1]).ok()?.duration();
    let meta = serde_json::from_str(&std::fs::read_to_string(&files[0]).ok()?).ok()?;
    Some((wav, meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus_config(dir: &Path) -> crate::config::Config {
        crate::config::Config {
            live_preview_corpus_dir: Some(dir.display().to_string()),
            live_preview_model: Some("live".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_cancelled_recording_is_saved_with_its_texts() {
        let dir = tempfile::TempDir::new().unwrap();
        let texts = CancelledTexts {
            preview_text: "open the file".into(),
            final_parts: vec!["Open the".into()],
        };
        save_cancelled(&corpus_config(dir.path()), vec![0.1; 8000], &texts).expect("a save")();
        let (len, meta) = only_recording(dir.path()).expect("a recording");
        assert_eq!(len, 8000);
        assert_eq!(meta["outcome"], "cancelled");
        assert_eq!(meta["preview_text"], "open the file");
        assert_eq!(meta["final_parts"], serde_json::json!(["Open the"]));
        assert!(meta["final_text"].is_null());
        assert_eq!(meta["seconds"], 0.5);
        assert_eq!(meta["preview_model"], "live");
    }

    #[test]
    fn nothing_is_saved_without_audio_or_a_corpus_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let texts = CancelledTexts::default();
        assert!(save_cancelled(&corpus_config(dir.path()), vec![], &texts).is_none());
        let mut config = corpus_config(dir.path());
        config.live_preview_corpus_dir = None;
        assert!(save_cancelled(&config, vec![0.1; 8000], &texts).is_none());
    }
}
