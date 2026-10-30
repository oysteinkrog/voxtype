//! Never lose a dictation to a failed worker.
//!
//! When transcription fails (the worker could not start, or died before its
//! final text), the recorded audio is still in the daemon's memory. It is
//! saved as a WAV file at once, and transcribed once more with a fresh
//! worker before the daemon gives up.

use super::Transcriber;
use crate::error::TranscribeError;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where a failed dictation's audio goes: `<corpus>/failed/` when
/// `live_preview_corpus_dir` is set, else `$XDG_STATE_HOME/voxtype/failed/`.
pub fn failed_audio_dir(corpus_dir: Option<&str>) -> PathBuf {
    match corpus_dir {
        Some(dir) => super::revisions::expand_home(dir).join("failed"),
        None => dirs::state_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("voxtype")
            .join("failed"),
    }
}

/// Write 16 kHz mono samples to `<dir>/<time>.wav`.
pub fn save_wav(dir: &Path, samples: &[f32]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    let path = dir.join(format!("{}.wav", stamp));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let write = || -> Result<(), hound::Error> {
        let mut w = hound::WavWriter::create(&path, spec)?;
        for &s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        w.finalize()
    };
    write().map_err(std::io::Error::other)?;
    Ok(path)
}

/// Transcribe `samples`; if that fails (or `failed` says the first try
/// already failed elsewhere, e.g. the live stream), save the audio to `dir`
/// and try once more. The saved file's path goes into `saved`, so the
/// caller can say where it is, whether or not the retry worked.
pub fn transcribe_or_rescue(
    transcriber: &dyn Transcriber,
    samples: &[f32],
    failed: Option<&str>,
    dir: &Path,
    saved: &Mutex<Option<PathBuf>>,
) -> Result<String, TranscribeError> {
    let first = match failed {
        Some(e) => e.to_string(),
        None => match transcriber.transcribe(samples) {
            Ok(text) => return Ok(text),
            Err(e) => e.to_string(),
        },
    };
    tracing::warn!(
        "Transcription failed ({}); saving the audio and trying once more",
        first
    );
    match save_wav(dir, samples) {
        Ok(path) => {
            tracing::info!("Saved the dictation's audio to {}", path.display());
            if let Ok(mut s) = saved.lock() {
                *s = Some(path);
            }
        }
        Err(e) => tracing::error!(
            "Could not save the dictation's audio to {}: {}",
            dir.display(),
            e
        ),
    }
    let retry = transcriber.transcribe(samples);
    match &retry {
        Ok(_) => tracing::info!("The retry worked"),
        Err(e) => tracing::error!("The retry failed too: {}", e),
    }
    retry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Fails its first `fails` calls, like a worker that cannot start.
    struct Flaky {
        fails: u32,
        calls: AtomicU32,
    }

    impl Transcriber for Flaky {
        fn transcribe(&self, _: &[f32]) -> Result<String, TranscribeError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fails {
                Err(TranscribeError::InitFailed(
                    "Failed to spawn transcribe-worker: No such file or directory".into(),
                ))
            } else {
                Ok("hello".into())
            }
        }
    }

    fn clip() -> Vec<f32> {
        (0..16000).map(|i| (i as f32 / 50.0).sin() * 0.5).collect()
    }

    #[test]
    fn a_working_transcriber_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let t = Flaky {
            fails: 0,
            calls: AtomicU32::new(0),
        };
        let saved = Mutex::new(None);
        let text = transcribe_or_rescue(&t, &clip(), None, dir.path(), &saved).unwrap();
        assert_eq!(text, "hello");
        assert!(saved.lock().unwrap().is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn one_failure_saves_the_audio_and_the_retry_delivers() {
        let dir = tempfile::tempdir().unwrap();
        let t = Flaky {
            fails: 1,
            calls: AtomicU32::new(0),
        };
        let saved = Mutex::new(None);
        let text = transcribe_or_rescue(&t, &clip(), None, dir.path(), &saved).unwrap();
        assert_eq!(text, "hello");
        assert_eq!(t.calls.load(Ordering::SeqCst), 2);
        let path = saved.lock().unwrap().clone().expect("audio saved");
        let wav = hound::WavReader::open(&path).unwrap();
        assert_eq!(wav.spec().sample_rate, 16000);
        assert_eq!(wav.len(), 16000);
    }

    #[test]
    fn two_failures_give_up_but_keep_the_audio() {
        let dir = tempfile::tempdir().unwrap();
        let t = Flaky {
            fails: 2,
            calls: AtomicU32::new(0),
        };
        let saved = Mutex::new(None);
        assert!(transcribe_or_rescue(&t, &clip(), None, dir.path(), &saved).is_err());
        assert_eq!(t.calls.load(Ordering::SeqCst), 2, "one retry only");
        assert!(saved.lock().unwrap().as_ref().unwrap().exists());
    }

    #[test]
    fn a_failed_live_stream_goes_straight_to_save_and_retry() {
        let dir = tempfile::tempdir().unwrap();
        let t = Flaky {
            fails: 0,
            calls: AtomicU32::new(0),
        };
        let saved = Mutex::new(None);
        let text = transcribe_or_rescue(
            &t,
            &clip(),
            Some("worker ended without a final text"),
            dir.path(),
            &saved,
        )
        .unwrap();
        assert_eq!(text, "hello");
        assert_eq!(t.calls.load(Ordering::SeqCst), 1);
        assert!(saved.lock().unwrap().is_some());
    }

    #[test]
    fn failed_audio_goes_under_the_corpus_when_there_is_one() {
        assert_eq!(
            failed_audio_dir(Some("/data/corpus")),
            PathBuf::from("/data/corpus/failed")
        );
        assert!(failed_audio_dir(None).ends_with("voxtype/failed"));
    }
}
