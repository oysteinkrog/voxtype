//! Audio feedback module
//!
//! Provides audio cues (beeps/sounds) for recording start/stop events.
//! Supports multiple sound themes and custom sound files.

use crate::config::AudioFeedbackConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::source::UniformSourceIterator;
use rodio::{Decoder, Source};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Sound event types
#[derive(Debug, Clone, Copy)]
pub enum SoundEvent {
    /// Recording started
    RecordingStart,
    /// Recording stopped
    RecordingStop,
    /// Transcription completed and text output successfully
    TranscriptionComplete,
    /// Recording/transcription cancelled
    Cancelled,
    /// Error occurred
    Error,
}

/// Audio feedback player
///
/// Each sound opens its own cpal output stream, plays, and closes it. The
/// stream's error callback only sets a flag, playback stops at the first
/// error, and one warning is logged. rodio's long-lived stream used cpal's
/// default callback, which printed every `alsa::poll()` POLLERR: on an
/// output that breaks (a remote desktop sink, a USB DAC after an underrun)
/// that was about 100k log lines a minute and most of a CPU core, and it
/// made the whole desktop lag.
pub struct AudioFeedback {
    config: AudioFeedbackConfig,
    theme: SoundTheme,
}

/// A sound theme containing audio data for different events
struct SoundTheme {
    start: Vec<u8>,
    stop: Vec<u8>,
    complete: Vec<u8>,
    cancel: Vec<u8>,
    error: Vec<u8>,
}

impl AudioFeedback {
    /// Create a new audio feedback player
    pub fn new(config: &AudioFeedbackConfig) -> Result<Self, String> {
        if !config.enabled {
            return Err("Audio feedback is disabled".to_string());
        }

        // Check once so a missing output device is still reported at startup.
        cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "Failed to open audio output: no output device".to_string())?;

        let theme = load_theme(&config.theme)?;

        Ok(Self {
            config: config.clone(),
            theme,
        })
    }

    /// Play a sound for the given event
    pub fn play(&self, event: SoundEvent) {
        let sound_data = match event {
            SoundEvent::RecordingStart => &self.theme.start,
            SoundEvent::RecordingStop => &self.theme.stop,
            SoundEvent::TranscriptionComplete => &self.theme.complete,
            SoundEvent::Cancelled => &self.theme.cancel,
            SoundEvent::Error => &self.theme.error,
        };

        if sound_data.is_empty() {
            return;
        }

        if let Err(e) = self.play_wav(sound_data) {
            tracing::warn!("Failed to play feedback sound: {}", e);
        }
    }

    fn play_wav(&self, data: &[u8]) -> Result<(), String> {
        let cursor = Cursor::new(data.to_vec());
        let source = Decoder::new(cursor).map_err(|e| format!("Failed to decode audio: {}", e))?;

        // Apply volume control
        let source = source.amplify(self.config.volume);

        // The stream lives on a short thread (cpal streams are not Send).
        std::thread::Builder::new()
            .name("voxtype-feedback".into())
            .spawn(move || {
                if let Err(e) = play_on_default_output(source) {
                    tracing::warn!("Failed to play feedback sound: {}", e);
                }
            })
            .map_err(|e| format!("Failed to start feedback thread: {}", e))?;

        Ok(())
    }
}

/// Play `source` once on the default output and close the stream. Returns
/// after the last sample, at the first stream error, or after the sound's
/// length plus one second, whichever comes first.
fn play_on_default_output<S>(source: S) -> Result<(), String>
where
    S: Source<Item = i16> + Send + 'static,
{
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("no output config: {e}"))?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let samples: Arc<Vec<f32>> = Arc::new(
        UniformSourceIterator::<_, f32>::new(source, config.channels, config.sample_rate.0)
            .collect(),
    );
    let length = Duration::from_secs_f64(
        samples.len() as f64 / (config.channels as f64 * config.sample_rate.0 as f64),
    );

    let pos = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let err_flag = failed.clone();
    let on_error = move |e: cpal::StreamError| {
        if !err_flag.swap(true, Ordering::Relaxed) {
            tracing::warn!("Feedback sound stopped: output stream error: {e}");
        }
    };
    let (s, p) = (samples.clone(), pos.clone());
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |out: &mut [f32], _| fill(out, &s, &p, |v| v),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |out: &mut [i16], _| fill(out, &s, &p, |v| (v * i16::MAX as f32) as i16),
            on_error,
            None,
        ),
        other => return Err(format!("unsupported output format {other:?}")),
    }
    .map_err(|e| format!("cannot open output stream: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("cannot start output stream: {e}"))?;

    let started = Instant::now();
    let deadline = started + length + Duration::from_secs(1);
    // The sound server pulls samples ahead of playback, so the stream stays
    // open for at least the sound's length plus a short drain.
    let heard = started + length + Duration::from_millis(80);
    while (pos.load(Ordering::Relaxed) < samples.len() || Instant::now() < heard)
        && !failed.load(Ordering::Relaxed)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    Ok(())
}

/// Copy the next samples into `out`, converted by `conv`; silence after the end.
fn fill<T: Copy + Default>(
    out: &mut [T],
    samples: &[f32],
    pos: &AtomicUsize,
    conv: impl Fn(f32) -> T,
) {
    let start = pos.load(Ordering::Relaxed);
    for (i, o) in out.iter_mut().enumerate() {
        *o = samples.get(start + i).map(|v| conv(*v)).unwrap_or_default();
    }
    pos.store((start + out.len()).min(samples.len()), Ordering::Relaxed);
}

/// Load a sound theme by name or path
fn load_theme(theme_name: &str) -> Result<SoundTheme, String> {
    match theme_name {
        "default" => Ok(generate_default_theme()),
        "subtle" => Ok(generate_subtle_theme()),
        "mechanical" => Ok(generate_mechanical_theme()),
        path => load_custom_theme(path),
    }
}

/// Load a custom theme from a directory
fn load_custom_theme(path: &str) -> Result<SoundTheme, String> {
    let dir = PathBuf::from(path);
    if !dir.is_dir() {
        return Err(format!("Theme directory not found: {}", path));
    }

    let load_file = |name: &str| -> Vec<u8> {
        let file_path = dir.join(name);
        std::fs::read(&file_path).unwrap_or_default()
    };

    Ok(SoundTheme {
        start: load_file("start.wav"),
        stop: load_file("stop.wav"),
        complete: load_file("complete.wav"),
        cancel: load_file("cancel.wav"),
        error: load_file("error.wav"),
    })
}

// === Sound Generation ===
// Generate simple WAV sounds programmatically to avoid shipping binary assets

/// Generate a simple WAV file with a sine wave tone
fn generate_tone_wav(frequency: f32, duration_ms: u32, fade_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;
    let fade_samples = (sample_rate * fade_ms / 1000) as usize;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let mut amplitude = (2.0 * std::f32::consts::PI * frequency * t).sin();

        // Apply fade in/out envelope
        if i < fade_samples {
            amplitude *= i as f32 / fade_samples as f32;
        } else if i >= num_samples - fade_samples {
            amplitude *= (num_samples - i) as f32 / fade_samples as f32;
        }

        samples.push((amplitude * 16000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Generate a two-tone sound (rising or falling)
fn generate_two_tone_wav(freq1: f32, freq2: f32, duration_ms: u32, fade_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;
    let fade_samples = (sample_rate * fade_ms / 1000) as usize;
    let half_samples = num_samples / 2;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let freq = if i < half_samples { freq1 } else { freq2 };
        let mut amplitude = (2.0 * std::f32::consts::PI * freq * t).sin();

        // Apply fade in/out envelope
        if i < fade_samples {
            amplitude *= i as f32 / fade_samples as f32;
        } else if i >= num_samples - fade_samples {
            amplitude *= (num_samples - i) as f32 / fade_samples as f32;
        }

        samples.push((amplitude * 16000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Generate a click sound (short burst of noise with envelope)
fn generate_click_wav(duration_ms: u32) -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = (sample_rate * duration_ms / 1000) as usize;

    let mut samples: Vec<i16> = Vec::with_capacity(num_samples);

    for i in 0..num_samples {
        // Quick exponential decay envelope
        let envelope = (-5.0 * i as f32 / num_samples as f32).exp();
        // High-frequency noise burst
        let noise = if i % 2 == 0 { 1.0 } else { -1.0 };
        samples.push((noise * envelope * 12000.0) as i16);
    }

    encode_wav(&samples, sample_rate)
}

/// Encode samples as WAV format
fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let mut wav = Vec::new();

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    let file_size = (36 + samples.len() * 2) as u32;
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&((samples.len() * 2) as u32).to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    wav
}

// === Built-in Themes ===

/// Default theme: Clear, pleasant tones
fn generate_default_theme() -> SoundTheme {
    SoundTheme {
        // Rising two-tone: 440Hz -> 880Hz (musical, energizing)
        start: generate_two_tone_wav(440.0, 880.0, 150, 20),
        // Falling two-tone: 880Hz -> 440Hz (completion)
        stop: generate_two_tone_wav(880.0, 440.0, 150, 20),
        // High ping: short 1200Hz tone (distinct from start/stop two-tones)
        complete: generate_tone_wav(1200.0, 80, 15),
        // Quick descending triple-beep for cancel (distinct from stop)
        cancel: generate_tone_wav(600.0, 80, 10),
        // Low warning tone
        error: generate_two_tone_wav(300.0, 200.0, 200, 30),
    }
}

/// Subtle theme: Quiet, unobtrusive clicks
fn generate_subtle_theme() -> SoundTheme {
    SoundTheme {
        // Soft high click
        start: generate_tone_wav(1200.0, 50, 10),
        // Soft low click
        stop: generate_tone_wav(800.0, 50, 10),
        // Gentle rising two-tone pip
        complete: generate_two_tone_wav(900.0, 1100.0, 60, 10),
        // Quick mid-tone for cancel
        cancel: generate_tone_wav(600.0, 40, 8),
        // Double low click
        error: generate_two_tone_wav(400.0, 300.0, 100, 15),
    }
}

/// Mechanical theme: Typewriter/keyboard-like sounds
fn generate_mechanical_theme() -> SoundTheme {
    SoundTheme {
        // Sharp click
        start: generate_click_wav(30),
        // Softer click
        stop: generate_click_wav(20),
        // Carriage return bell
        complete: generate_tone_wav(2000.0, 40, 8),
        // Double click for cancel
        cancel: generate_click_wav(15),
        // Buzzer
        error: generate_tone_wav(150.0, 150, 20),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a sound server. Run it against a silent sink, e.g.
    /// `pactl load-module module-null-sink sink_name=vxtest` and
    /// `PIPEWIRE_NODE=vxtest cargo test -- --ignored plays_and_closes`.
    #[test]
    #[ignore]
    fn plays_and_closes() {
        let wav = generate_tone_wav(880.0, 120, 5);
        let source = Decoder::new(Cursor::new(wav)).unwrap();
        let started = Instant::now();
        play_on_default_output(source).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn test_generate_tone_wav() {
        let wav = generate_tone_wav(440.0, 100, 10);
        // Check WAV header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(!wav.is_empty());
    }

    #[test]
    fn test_generate_themes() {
        let default = generate_default_theme();
        assert!(!default.start.is_empty());
        assert!(!default.stop.is_empty());
        assert!(!default.complete.is_empty());
        assert!(!default.cancel.is_empty());
        assert!(!default.error.is_empty());

        let subtle = generate_subtle_theme();
        assert!(!subtle.start.is_empty());
        assert!(!subtle.complete.is_empty());
        assert!(!subtle.cancel.is_empty());

        let mechanical = generate_mechanical_theme();
        assert!(!mechanical.start.is_empty());
        assert!(!mechanical.complete.is_empty());
        assert!(!mechanical.cancel.is_empty());
    }
}
