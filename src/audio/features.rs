//! Voice features for each 10 ms level frame, for OSD visuals.
//!
//! The analyzer keeps the last 32 ms of audio (512 samples at 16 kHz) and,
//! once per 10 ms hop, measures:
//!
//! - loudness: RMS over the last 25 ms, in dBFS;
//! - pitch: a YIN estimate of F0 from 64 to 500 Hz, and whether the frame is
//!   voiced;
//! - brightness: the spectral centroid in Hz;
//! - noisiness: the spectral flatness, 0 (tonal) to 1 (white noise);
//! - a coarse spectrum: 24 mel bands from 50 Hz to 8 kHz, as bytes;
//! - onset strength: how fast the mel bands rise, 0 to 1. A value of 0.5 or
//!   more marks an onset, roughly a syllable or word start, at most one per
//!   50 ms.
//!
//! Everything is causal: a frame only uses audio up to its own end. The
//! work is a 512-point FFT and a YIN difference function per hop, with no
//! allocation after construction.

use std::f32::consts::PI;

use super::levels::{SAMPLES_PER_FRAME, SAMPLE_RATE};

/// Samples the analysis looks at: 32 ms.
const WINDOW: usize = 512;
/// Samples the RMS covers: 25 ms.
const RMS_SAMPLES: usize = 400;
/// Mel bands in a frame.
pub const MEL_BANDS: usize = 24;
const MEL_MIN_HZ: f32 = 50.0;
const MEL_MAX_HZ: f32 = 8000.0;
/// A mel band byte is (dB + MEL_DB_RANGE) scaled so 0 dB is 255 and
/// -MEL_DB_RANGE dB or lower is 0.
pub const MEL_DB_RANGE: f32 = 100.0;
/// Pitch search range. The low end is set by the window: YIN needs the lag
/// plus an integration span of at least as many samples.
const F0_MIN_HZ: f32 = 64.0;
const F0_MAX_HZ: f32 = 500.0;
/// YIN's cumulative mean normalised difference must dip below this for a
/// voiced frame. Against pyin on recorded dictation, 0.3 found 70% of the
/// voiced frames with 88% precision; 0.15 found only 40%.
const YIN_THRESHOLD: f32 = 0.3;
/// How close to the deepest dip an earlier dip may be and still win.
const YIN_TOLERANCE: f32 = 0.02;
/// Frames quieter than this are never voiced.
const VOICED_MIN_DBFS: f32 = -55.0;
/// Onset strength compares each mel band with the frame this many hops
/// back (20 ms), and reads 1 when the bands rise by `ONSET_FULL_DB` on
/// average. Frames below `ONSET_MIN_DBFS` have none. Against the syllable
/// nuclei of three recorded dictations (72 syllables), 78% of syllables had
/// an onset in the 120 ms before their nucleus and 63% of onsets had a
/// nucleus in the 120 ms after, at 1.45 onsets per syllable. Onsets at
/// random times in speech scored 48% and 46%.
const ONSET_LAG: usize = 2;
const ONSET_FULL_DB: f32 = 6.0;
const ONSET_MIN_DBFS: f32 = -50.0;
/// After an onset, strength stays below 0.5 for this many hops (50 ms).
const ONSET_REFRACTORY: u32 = 5;

/// The voice features of one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFeatures {
    /// RMS of the last 25 ms in dBFS, -120 for silence.
    pub rms_dbfs: f32,
    /// Fundamental frequency in Hz, 0 when not voiced.
    pub f0_hz: f32,
    pub voiced: bool,
    /// Spectral centroid in Hz, 0 for silence.
    pub centroid_hz: f32,
    /// Spectral flatness, 0 to 1.
    pub flatness: f32,
    /// Mel band levels, 0 for -100 dB or less and 255 for 0 dB.
    pub mel: [u8; MEL_BANDS],
    /// Onset strength, 0 to 1; 0.5 or more is an onset.
    pub onset: f32,
}

/// Turns the sample stream into one [`AudioFeatures`] per 10 ms hop.
pub struct FeatureAnalyzer {
    /// The last `WINDOW` samples, oldest first after `ordered`.
    ring: [f32; WINDOW],
    pos: usize,
    /// Samples since the last hop.
    hop: usize,
    hann: [f32; WINDOW],
    /// Bit-reversed index order and twiddles for the FFT.
    rev: [u16; WINDOW],
    twiddle: [(f32, f32); WINDOW / 2],
    /// Per band: first FFT bin and the triangle weights from there.
    mel: Vec<(usize, Vec<f32>)>,
    frame: [f32; WINDOW],
    re: [f32; WINDOW],
    im: [f32; WINDOW],
    power: [f32; WINDOW / 2 + 1],
    diff: Vec<f32>,
    /// Mel band levels in dB of the last `ONSET_LAG` hops, newest last.
    mel_db: [[f32; MEL_BANDS]; ONSET_LAG],
    /// Hops left in which no new onset may start.
    refractory: u32,
}

impl Default for FeatureAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

fn dbfs(power: f32) -> f32 {
    if power <= 1e-12 {
        -120.0
    } else {
        10.0 * power.log10()
    }
}

impl FeatureAnalyzer {
    pub fn new() -> Self {
        let mut hann = [0.0; WINDOW];
        for (i, h) in hann.iter_mut().enumerate() {
            *h = 0.5 - 0.5 * (2.0 * PI * i as f32 / WINDOW as f32).cos();
        }
        let bits = WINDOW.trailing_zeros();
        let mut rev = [0u16; WINDOW];
        for (i, r) in rev.iter_mut().enumerate() {
            *r = ((i as u32).reverse_bits() >> (32 - bits)) as u16;
        }
        let mut twiddle = [(0.0, 0.0); WINDOW / 2];
        for (k, t) in twiddle.iter_mut().enumerate() {
            let a = -2.0 * PI * k as f32 / WINDOW as f32;
            *t = (a.cos(), a.sin());
        }
        // Triangular mel filters, each normalised to unit area so a flat
        // spectrum gives the same level in every band.
        let bin_hz = SAMPLE_RATE as f32 / WINDOW as f32;
        let (lo, hi) = (hz_to_mel(MEL_MIN_HZ), hz_to_mel(MEL_MAX_HZ));
        let edges: Vec<f32> = (0..MEL_BANDS + 2)
            .map(|i| mel_to_hz(lo + (hi - lo) * i as f32 / (MEL_BANDS + 1) as f32) / bin_hz)
            .collect();
        let mel = (0..MEL_BANDS)
            .map(|b| {
                let (l, c, r) = (edges[b], edges[b + 1], edges[b + 2]);
                let first = l.floor().max(0.0) as usize;
                let last = (r.ceil() as usize).min(WINDOW / 2);
                let mut w: Vec<f32> = (first..=last)
                    .map(|k| {
                        let k = k as f32;
                        if k <= c {
                            ((k - l) / (c - l)).max(0.0)
                        } else {
                            ((r - k) / (r - c)).max(0.0)
                        }
                    })
                    .collect();
                // Narrow low bands can fall between bins; give them the
                // nearest bin instead of nothing.
                let sum: f32 = w.iter().sum();
                if sum < 1e-6 {
                    w.iter_mut().for_each(|x| *x = 0.0);
                    let nearest = (c.round() as usize - first).min(w.len() - 1);
                    w[nearest] = 1.0;
                } else {
                    w.iter_mut().for_each(|x| *x /= sum);
                }
                (first, w)
            })
            .collect();
        let max_lag = (SAMPLE_RATE as f32 / F0_MIN_HZ) as usize;
        Self {
            ring: [0.0; WINDOW],
            pos: 0,
            hop: 0,
            hann,
            rev,
            twiddle,
            mel,
            frame: [0.0; WINDOW],
            re: [0.0; WINDOW],
            im: [0.0; WINDOW],
            power: [0.0; WINDOW / 2 + 1],
            diff: vec![0.0; max_lag + 1],
            mel_db: [[-MEL_DB_RANGE; MEL_BANDS]; ONSET_LAG],
            refractory: 0,
        }
    }

    /// Adds one sample. Every `SAMPLES_PER_FRAME` samples, returns the
    /// features of the hop that just ended.
    #[inline]
    pub fn push(&mut self, sample: f32) -> Option<AudioFeatures> {
        self.ring[self.pos] = sample;
        self.pos = (self.pos + 1) % WINDOW;
        self.hop += 1;
        if self.hop < SAMPLES_PER_FRAME {
            return None;
        }
        self.hop = 0;
        Some(self.analyze())
    }

    fn analyze(&mut self) -> AudioFeatures {
        let (a, b) = self.ring.split_at(self.pos);
        self.frame[..b.len()].copy_from_slice(b);
        self.frame[b.len()..].copy_from_slice(a);

        let tail = &self.frame[WINDOW - RMS_SAMPLES..];
        let rms_dbfs = dbfs(tail.iter().map(|x| x * x).sum::<f32>() / RMS_SAMPLES as f32);
        let f0_hz = if rms_dbfs >= VOICED_MIN_DBFS {
            self.yin()
        } else {
            0.0
        };

        self.spectrum();
        let bin_hz = SAMPLE_RATE as f32 / WINDOW as f32;
        let bins = &self.power[1..];
        let total: f32 = bins.iter().sum();
        let (centroid_hz, flatness) = if total <= 1e-10 {
            (0.0, 0.0)
        } else {
            let centroid = bins
                .iter()
                .enumerate()
                .map(|(i, p)| (i + 1) as f32 * bin_hz * p)
                .sum::<f32>()
                / total;
            let mean = total / bins.len() as f32;
            let log_mean = bins.iter().map(|p| (p + 1e-12).ln()).sum::<f32>() / bins.len() as f32;
            (centroid, (log_mean.exp() / mean).clamp(0.0, 1.0))
        };

        let mut mel = [0u8; MEL_BANDS];
        let mut mel_db = [0.0; MEL_BANDS];
        for ((m, d), (first, w)) in mel.iter_mut().zip(&mut mel_db).zip(&self.mel) {
            let p: f32 = w
                .iter()
                .zip(&self.power[*first..])
                .map(|(w, p)| w * p)
                .sum();
            *d = dbfs(p).max(-MEL_DB_RANGE);
            *m = ((*d + MEL_DB_RANGE) / MEL_DB_RANGE * 255.0).clamp(0.0, 255.0) as u8;
        }
        let onset = self.onset(&mel_db, rms_dbfs);

        AudioFeatures {
            rms_dbfs,
            f0_hz,
            voiced: f0_hz > 0.0,
            centroid_hz,
            flatness,
            mel,
            onset,
        }
    }

    /// Spectral flux: the mean rise of the mel bands over `ONSET_LAG` hops,
    /// scaled so `ONSET_FULL_DB` reads 1, with a refractory time after each
    /// onset.
    fn onset(&mut self, mel_db: &[f32; MEL_BANDS], rms_dbfs: f32) -> f32 {
        let rise = mel_db
            .iter()
            .zip(&self.mel_db[0])
            .map(|(now, then)| (now - then).max(0.0))
            .sum::<f32>()
            / MEL_BANDS as f32;
        self.mel_db.rotate_left(1);
        self.mel_db[ONSET_LAG - 1] = *mel_db;
        let mut strength = if rms_dbfs < ONSET_MIN_DBFS {
            0.0
        } else {
            (rise / ONSET_FULL_DB).clamp(0.0, 1.0)
        };
        if self.refractory > 0 {
            self.refractory -= 1;
            strength = strength.min(0.49);
        } else if strength >= 0.5 {
            self.refractory = ONSET_REFRACTORY - 1;
        }
        strength
    }

    /// YIN on the window. The frame is voiced when the cumulative mean
    /// normalised difference dips below the threshold anywhere in range; the
    /// pitch is then the shortest lag at a local minimum within
    /// `YIN_TOLERANCE` of the deepest one, refined by a parabola. Taking the
    /// first dip under the threshold instead, as plain YIN does, often lands
    /// on a harmonic an octave too high. 0 when not voiced.
    fn yin(&mut self) -> f32 {
        let x = &self.frame;
        let max_lag = self.diff.len() - 1;
        let min_lag = (SAMPLE_RATE as f32 / F0_MAX_HZ) as usize;
        let span = WINDOW - max_lag;
        self.diff[0] = 1.0;
        let mut running = 0.0;
        for lag in 1..=max_lag {
            let d: f32 = (0..span).map(|j| (x[j] - x[j + lag]).powi(2)).sum();
            running += d;
            self.diff[lag] = if running > 0.0 {
                d * lag as f32 / running
            } else {
                1.0
            };
        }
        let c = &self.diff;
        let deepest = c[min_lag..max_lag]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        if deepest >= YIN_THRESHOLD {
            return 0.0;
        }
        let Some(lag) = (min_lag..max_lag)
            .find(|&l| c[l] <= deepest + YIN_TOLERANCE && c[l] <= c[l - 1] && c[l] <= c[l + 1])
        else {
            return 0.0;
        };
        let (a, b, d) = (c[lag - 1], c[lag], c[lag + 1]);
        let den = a - 2.0 * b + d;
        let shift = if den.abs() > 1e-9 {
            (0.5 * (a - d) / den).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        SAMPLE_RATE as f32 / (lag as f32 + shift)
    }

    /// Power spectrum of the Hann-windowed frame, in `power`, scaled so a
    /// full-scale sine reads about 0 dB in its bin.
    fn spectrum(&mut self) {
        for i in 0..WINDOW {
            let j = self.rev[i] as usize;
            self.re[j] = self.frame[i] * self.hann[i];
            self.im[j] = 0.0;
        }
        let mut len = 2;
        while len <= WINDOW {
            let step = WINDOW / len;
            for start in (0..WINDOW).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.twiddle[k * step];
                    let (i, j) = (start + k, start + k + len / 2);
                    let tr = self.re[j] * wr - self.im[j] * wi;
                    let ti = self.re[j] * wi + self.im[j] * wr;
                    self.re[j] = self.re[i] - tr;
                    self.im[j] = self.im[i] - ti;
                    self.re[i] += tr;
                    self.im[i] += ti;
                }
            }
            len *= 2;
        }
        // A Hann window has a coherent gain of 0.5, so a sine of amplitude 1
        // peaks at WINDOW / 4 in magnitude.
        let scale = 4.0 / WINDOW as f32;
        for (k, p) in self.power.iter_mut().enumerate() {
            let (r, i) = (self.re[k] * scale, self.im[k] * scale);
            *p = r * r + i * i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(signal: impl Fn(usize) -> f32, secs: f32) -> Vec<AudioFeatures> {
        let mut a = FeatureAnalyzer::new();
        (0..(secs * SAMPLE_RATE as f32) as usize)
            .filter_map(|n| a.push(signal(n)))
            .collect()
    }

    fn sine(hz: f32, amp: f32) -> impl Fn(usize) -> f32 {
        move |n| amp * (2.0 * PI * hz * n as f32 / SAMPLE_RATE as f32).sin()
    }

    /// A voice-like tone: a fundamental and decaying harmonics.
    fn voice(hz: f32) -> impl Fn(usize) -> f32 {
        move |n| {
            (1..=8)
                .map(|h| {
                    0.3 / h as f32
                        * (2.0 * PI * hz * h as f32 * n as f32 / SAMPLE_RATE as f32).sin()
                })
                .sum()
        }
    }

    #[test]
    fn a_tone_burst_is_one_onset() {
        // 300 ms of silence, then a voice; a steady tone after that.
        let v = voice(150.0);
        let f = run(move |n| if n < 4800 { 0.0 } else { v(n) }, 0.8);
        let onsets: Vec<usize> = (0..f.len()).filter(|&i| f[i].onset >= 0.5).collect();
        assert_eq!(onsets.len(), 1, "onsets at {onsets:?}");
        assert!(
            (30..34).contains(&onsets[0]),
            "onset at frame {}",
            onsets[0]
        );
        assert!(
            f[60..].iter().all(|x| x.onset < 0.1),
            "steady tone has no onset"
        );
        assert!(run(|_| 0.0, 0.3).iter().all(|x| x.onset == 0.0));
    }

    #[test]
    fn onsets_keep_50_ms_apart() {
        // Bursts every 30 ms: only every other one may count.
        let v = voice(150.0);
        let f = run(move |n| if (n / 240) % 2 == 1 { v(n) } else { 0.0 }, 1.0);
        let onsets: Vec<usize> = (0..f.len()).filter(|&i| f[i].onset >= 0.5).collect();
        assert!(onsets.len() >= 3, "onsets at {onsets:?}");
        assert!(
            onsets.windows(2).all(|w| w[1] - w[0] >= 5),
            "onsets at {onsets:?}"
        );
    }

    #[test]
    fn one_frame_per_hop() {
        assert_eq!(run(|_| 0.0, 1.0).len(), 100);
    }

    #[test]
    fn silence_is_quiet_unvoiced_and_dark() {
        let f = *run(|_| 0.0, 0.2).last().unwrap();
        assert_eq!(f.rms_dbfs, -120.0);
        assert!(!f.voiced);
        assert_eq!(f.f0_hz, 0.0);
        assert_eq!(f.centroid_hz, 0.0);
        assert_eq!(f.mel, [0; MEL_BANDS]);
    }

    #[test]
    fn yin_finds_the_pitch_of_a_voice() {
        for hz in [90.0, 140.0, 220.0, 330.0] {
            let f = *run(voice(hz), 0.2).last().unwrap();
            assert!(f.voiced, "{hz} Hz not voiced");
            assert!(
                (f.f0_hz - hz).abs() < hz * 0.02,
                "{hz} Hz read as {}",
                f.f0_hz
            );
        }
    }

    #[test]
    fn noise_is_unvoiced_bright_and_flat() {
        // splitmix64 of the sample index: white noise that repeats run to run.
        let noise = |n: usize| {
            let mut x = (n as u64).wrapping_add(0x9E3779B97F4A7C15);
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            x ^= x >> 31;
            ((x >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.5
        };
        let f = *run(noise, 0.2).last().unwrap();
        assert!(!f.voiced);
        assert!(f.flatness > 0.4, "flatness {}", f.flatness);
        assert!(f.centroid_hz > 3000.0, "centroid {}", f.centroid_hz);
        let tone = *run(sine(440.0, 0.5), 0.2).last().unwrap();
        assert!(tone.flatness < 0.05, "tone flatness {}", tone.flatness);
        assert!(
            (tone.centroid_hz - 440.0).abs() < 60.0,
            "tone centroid {}",
            tone.centroid_hz
        );
    }

    #[test]
    fn rms_and_mel_follow_the_level() {
        let f = *run(sine(1000.0, 1.0), 0.2).last().unwrap();
        // A full-scale sine has an RMS of -3 dBFS.
        assert!((f.rms_dbfs + 3.0).abs() < 0.2, "rms {}", f.rms_dbfs);
        let loudest = f.mel.iter().enumerate().max_by_key(|(_, &m)| m).unwrap().0;
        let quieter = *run(sine(1000.0, 0.01), 0.2).last().unwrap();
        assert_eq!(
            quieter
                .mel
                .iter()
                .enumerate()
                .max_by_key(|(_, &m)| m)
                .unwrap()
                .0,
            loudest
        );
        // 40 dB down is 102 steps on the 100 dB byte scale.
        let drop = f.mel[loudest] as i32 - quieter.mel[loudest] as i32;
        assert!((drop - 102).abs() <= 3, "drop {drop}");
        // 1 kHz sits in the middle of the mel range.
        assert!((6..16).contains(&loudest), "band {loudest}");
    }

    /// Writes the features of a 16 kHz mono WAV as CSV, to compare with an
    /// offline analysis. Run with `VOXTYPE_FEATURES_WAV=<in.wav>
    /// VOXTYPE_FEATURES_CSV=<out.csv> cargo test --lib audio::features --
    /// --ignored`.
    #[test]
    #[ignore]
    fn dump_wav() {
        let (Ok(wav), Ok(csv)) = (
            std::env::var("VOXTYPE_FEATURES_WAV"),
            std::env::var("VOXTYPE_FEATURES_CSV"),
        ) else {
            return;
        };
        let mut reader = hound::WavReader::open(wav).unwrap();
        let mut a = FeatureAnalyzer::new();
        let mut out = String::from("rms_dbfs,f0_hz,centroid_hz,flatness,mel,onset\n");
        for s in reader.samples::<i16>() {
            if let Some(f) = a.push(s.unwrap() as f32 / 32768.0) {
                let mel: Vec<String> = f.mel.iter().map(|m| m.to_string()).collect();
                out += &format!(
                    "{},{},{},{},{},{}\n",
                    f.rms_dbfs,
                    f.f0_hz,
                    f.centroid_hz,
                    f.flatness,
                    mel.join(" "),
                    f.onset
                );
            }
        }
        std::fs::write(csv, out).unwrap();
    }

    /// CPU per frame. Run with
    /// `cargo test --release --lib audio::features -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cost_per_frame() {
        let mut a = FeatureAnalyzer::new();
        let v = voice(150.0);
        let samples: Vec<f32> = (0..60 * SAMPLE_RATE as usize).map(v).collect();
        let start = std::time::Instant::now();
        let mut frames = 0;
        for &x in &samples {
            if let Some(f) = a.push(x) {
                std::hint::black_box(f);
                frames += 1;
            }
        }
        let per = start.elapsed().as_secs_f64() / frames as f64;
        println!(
            "features: {frames} frames, {:.1} us per frame, {:.2}% of one core at 100 Hz",
            per * 1e6,
            per * 100.0 * 100.0
        );
    }
}
