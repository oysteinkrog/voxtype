//! Moment scenarios (MO-07): short scripted dictations, each built round one
//! moment a look must handle well: the first word, a pause, a long silence, a
//! burst of revisions, a wrap, an overflow, the final pass, the deliveries,
//! the endings, a model load and the time limit.
//!
//! A moment is sent as the messages the daemon produces: audio frames with
//! voice features (onsets at syllables, pitch, level, mel bands) and the
//! runtime files, with word timing in `preview.json`. So it plays the same in
//! an offline render (`--offline DIR --moment NAME`), on the wall
//! (`--wall-script NAME`) and in a station tile.

use super::*;

use voxtype::osd::live::{LiveDoc, TimedWord};

/// One scripted moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Moment {
    /// The original demo: loading, words, a revision, a pause, the final text.
    Dictation,
    FirstWord,
    Pause,
    LongSilence,
    RevisionBurst,
    /// Words with pauses from 100 to 900 ms, a comma, a sentence end and
    /// one revision, for the kinetic lattice exhibit (MO-85).
    Lattice,
    Wrap,
    Overflow,
    FinalPass,
    Typed,
    PastedEnter,
    Cancel,
    NoSpeech,
    Failure,
    Loading,
    TimeLimit,
    /// A dictation with one press of the look chord in the middle.
    LookSwitch,
    /// The live model's steps on a fixture, with its lookahead tail and
    /// frame levels (`preview-signals.json`), for the model signal features
    /// (ASR-05).
    Lookahead,
    /// Two spoken corrections (COR-3): one placed, one with no match.
    Correction,
    /// A spelled correction (COR-5): the letters fill in while the key is
    /// held, then the word is placed and learned.
    Spell,
    /// A keyboard edit (plan M7.6): a tap in a pause, the caret up two
    /// lines, a word typed, Enter.
    Edit,
}

impl Moment {
    pub(crate) const ALL: &'static [Moment] = &[
        Moment::Dictation,
        Moment::FirstWord,
        Moment::Pause,
        Moment::LongSilence,
        Moment::RevisionBurst,
        Moment::Lattice,
        Moment::Wrap,
        Moment::Overflow,
        Moment::FinalPass,
        Moment::Typed,
        Moment::PastedEnter,
        Moment::Cancel,
        Moment::NoSpeech,
        Moment::Failure,
        Moment::Loading,
        Moment::TimeLimit,
        Moment::LookSwitch,
        Moment::Lookahead,
        Moment::Correction,
        Moment::Spell,
        Moment::Edit,
    ];

    /// The name on the command line.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Moment::Dictation => "dictation",
            Moment::FirstWord => "first-word",
            Moment::Pause => "pause",
            Moment::LongSilence => "long-silence",
            Moment::RevisionBurst => "revision-burst",
            Moment::Lattice => "lattice",
            Moment::Wrap => "wrap",
            Moment::Overflow => "overflow",
            Moment::FinalPass => "final-pass",
            Moment::Typed => "typed",
            Moment::PastedEnter => "pasted-enter",
            Moment::Cancel => "cancel",
            Moment::NoSpeech => "no-speech",
            Moment::Failure => "failure",
            Moment::Loading => "loading",
            Moment::TimeLimit => "time-limit",
            Moment::LookSwitch => "look-switch",
            Moment::Lookahead => "lookahead",
            Moment::Correction => "correction",
            Moment::Spell => "spell",
            Moment::Edit => "edit",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Moment> {
        let name = name.trim().to_ascii_lowercase().replace('_', "-");
        Moment::ALL.iter().copied().find(|m| m.name() == name)
    }

    /// Every name, for error messages and help.
    pub(crate) fn names() -> String {
        Moment::ALL
            .iter()
            .map(|m| m.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The demo sentence, in pieces. Moments take their words from here, so no
/// moment shows text of its own.
const SENTENCE: &str = "open the configuration file and set the popup theme to swiss so the \
    dictation window shows the live words in the middle of the screen with the waveform above \
    them then press the key again to stop and let the final pass fix the casing and the \
    punctuation before the text is typed into the editor";

fn sentence_words(from: usize, n: usize) -> Vec<&'static str> {
    SENTENCE.split_whitespace().skip(from).take(n).collect()
}

/// Seconds of room tone in the long-silence moment. Long enough for the
/// kinetic lattice to play all of its quiet out and then hold still: the
/// live strand slackens over `LIVE_SLACK` (3 s), the exhale starts
/// `EXHALE_AFTER` (3 s) into the silence and eases out over `EXHALE_OVER`
/// (1.2 s), and then the lattice rests for 2.3 s before the voice comes
/// back, so a render shows the rest as well as the motion (LT-12).
const LONG_SILENCE: f64 = 6.5;

/// The words of the lattice moment and the pause before each, in seconds.
/// The commas and full stops are in the text, so the lattice sees clause
/// and sentence ends.
const LATTICE_SCRIPT: &[(&str, f64)] = &[
    ("Open", 0.0),
    ("the", 0.10),
    ("configuration", 0.30),
    ("file,", 0.15),
    ("and", 0.55),
    ("set", 0.10),
    ("the", 0.20),
    ("popup", 0.90),
    ("theme", 0.35),
    ("to", 0.12),
    ("swish.", 0.25),
    ("Then", 0.70),
    ("press", 0.15),
    ("the", 0.10),
    ("key.", 0.45),
];

/// The lookahead moment (ASR-05, ASR-06): what parakeet-unified-en-0.6b-int8
/// streams on `tests/fixtures/vad/speech_long.wav` (dumped with parakeet-rs's
/// `unified_signals` example). Its words are the fixture's, not the demo
/// sentence's, because the tail and the doubt are the model's own.
type LookaheadToken = (&'static str, u32, f32, f32, f32, Option<&'static str>);
type LookaheadFrame = (f32, Option<(&'static str, f32)>);
type LookaheadStep = (usize, &'static [(&'static str, u32, f32)], &'static [LookaheadFrame]);

/// Its committed tokens, in order: (piece, frame, p, entropy,
/// top-2 margin, runner-up). A leading space starts a word.
const LOOKAHEAD_TOKENS: &[LookaheadToken] = &[
    (" Th", 7, 0.998, 0.017, 0.998, Some(" It")),
    ("is", 7, 0.999, 0.007, 0.999, None),
    (" is", 7, 0.948, 0.211, 0.896, None),
    (" a", 8, 0.993, 0.044, 0.988, None),
    (" l", 9, 0.998, 0.016, 0.997, None),
    ("ong", 9, 0.998, 0.015, 0.996, None),
    ("er", 10, 0.874, 0.390, 0.749, None),
    (" t", 13, 0.998, 0.025, 0.997, None),
    ("est", 13, 0.999, 0.007, 0.999, None),
    (" of", 16, 0.530, 0.724, 0.063, Some(" on")),
    (" v", 18, 0.998, 0.020, 0.997, None),
    ("o", 18, 0.999, 0.009, 0.999, None),
    ("ice", 18, 0.999, 0.010, 0.998, Some("ices")),
    (" act", 21, 0.941, 0.237, 0.883, None),
    ("iv", 23, 0.998, 0.018, 0.997, None),
    ("ity", 23, 0.991, 0.055, 0.983, None),
    (" det", 29, 0.999, 0.012, 0.998, None),
    ("ect", 30, 0.999, 0.010, 0.998, None),
    ("ion", 31, 0.992, 0.061, 0.987, None),
    (" w", 36, 0.995, 0.034, 0.991, None),
    ("ith", 36, 0.995, 0.033, 0.991, None),
    (" m", 38, 0.999, 0.010, 0.998, None),
    ("ult", 38, 0.999, 0.008, 0.998, None),
    ("i", 39, 0.998, 0.018, 0.997, None),
    ("ple", 39, 0.999, 0.011, 0.998, None),
    (" wor", 43, 1.000, 0.004, 1.000, None),
    ("ds", 43, 0.999, 0.005, 0.999, Some("d")),
    (" and", 51, 0.994, 0.040, 0.990, Some(",")),
    (" ph", 53, 0.998, 0.018, 0.998, None),
    ("r", 53, 1.000, 0.003, 1.000, Some("ases")),
    ("ases", 53, 1.000, 0.002, 1.000, None),
    (".", 57, 0.636, 0.657, 0.272, None),
];
/// Its steps: committed tokens so far, the tail as
/// (piece, frames to the window edge, p), and each new frame's
/// log10(1 - P(blank)) with its top non-blank guess (piece, ln p) where the
/// level is above -3. The last step is the flush.
const LOOKAHEAD_STEPS: &[LookaheadStep] = &[
    (0, &[(" Th", 7, 0.997), ("is", 7, 0.998), (" is", 7, 0.952), (" a", 6, 0.990), (" l", 5, 0.998), ("ong", 5, 0.999), ("er", 4, 0.983), (" t", 2, 0.990), ("est", 2, 0.998), (" on", 1, 0.887)], &[(-5.092, None), (-5.092, None), (-5.047, None), (-5.037, None), (-5.100, None), (-4.761, None), (-2.324, Some((" Th", -5.50)))]),
    (9, &[(" of", 5, 0.639), (" v", 4, 0.960), ("o", 3, 0.998), ("ice", 3, 0.995), (" act", 1, 0.995), ("ing", 0, 0.489), (".", 0, 0.994)], &[(-2.897, Some((" th", -7.60))), (-3.265, None), (-3.409, None), (-3.649, None), (-3.675, None), (-0.989, Some((" t", -2.30))), (-3.613, None)]),
    (13, &[(" act", 7, 0.995), ("iv", 6, 0.523), ("ity", 5, 0.988), (" det", 1, 0.957), ("ect", 0, 0.726), ("ion", 0, 0.331), (".", 0, 0.988)], &[(-4.071, None), (-2.474, Some((" of", -6.16))), (-3.145, None), (-1.500, Some((" v", -3.48))), (-4.117, None), (-3.974, None), (-4.279, None)]),
    (16, &[(" det", 6, 0.999), ("ect", 5, 0.999), ("ion", 4, 0.990), (" w", 2, 0.828), ("ith", 2, 0.939)], &[(-3.170, None), (-1.808, Some(("iv", -4.18))), (-4.052, None), (-3.962, None), (-4.296, None), (-4.500, None), (-4.527, None)]),
    (19, &[(" w", 6, 0.997), ("ith", 6, 0.995), (" m", 4, 0.999), ("ult", 4, 0.999), ("i", 3, 0.998), ("ple", 3, 0.998), (" wor", 1, 0.995), ("ds", 1, 0.870), (".", 0, 0.972)], &[(-4.645, None), (-2.017, Some(("ect", -4.67))), (-0.658, Some(("ion", -1.52))), (-4.304, None), (-3.923, None), (-3.955, None), (-3.951, None)]),
    (25, &[(" wor", 6, 1.000), ("ds", 6, 0.999), (",", 1, 0.793), (" and", 1, 0.922)], &[(-4.014, None), (-3.731, None), (-3.509, None), (-1.620, Some(("i", -3.73))), (-4.520, None), (-4.460, None), (-4.462, None)]),
    (27, &[(" and", 5, 0.991), (" ph", 4, 0.986), ("r", 3, 0.999), ("ases", 3, 1.000), (".", 0, 0.997)], &[(-4.635, None), (-4.149, None), (-4.036, None), (-4.132, None), (-4.035, None), (-3.969, None), (-3.558, None)]),
    (32, &[], &[(-1.913, Some((",", -4.41))), (-1.043, Some((" and", -2.64))), (-3.762, None), (-0.927, Some((" ph", -2.15))), (-4.604, None), (-2.106, Some((".", -4.86))), (-1.163, Some((".", -2.68))), (-0.527, Some((".", -1.21))), (-5.398, None), (-5.338, None), (-6.0, None)]),
];

/// The sound strip's phones at each step of the lookahead moment (ASR-10),
/// from the live worker with `sound-strip` on, on the same fixture: the
/// frames the step committed and the lookahead frames after them, each as
/// (likeliest phone, its probability).
type LookaheadSounds = (&'static [(&'static str, f32)], &'static [(&'static str, f32)]);
const LOOKAHEAD_SOUNDS: &[LookaheadSounds] = &[
    (&[("DH", 0.90), ("IH", 0.42), ("IH", 0.58), ("M", 0.46), ("AH", 0.96), ("L", 0.83), ("N", 0.45)], &[("AH", 0.69), ("AH", 0.92), ("AH", 0.63), ("EH", 0.95), ("T", 0.82), ("AA", 0.76), ("M", 0.93), ("M", 0.58)]),
    (&[("AH", 0.64), ("AH", 0.88), ("AH", 0.55), ("EH", 0.96), ("T", 0.72), ("AA", 0.91), ("M", 0.67)], &[("M", 0.99), ("OY", 0.85), ("OY", 0.89), ("AA", 0.49), ("K", 0.77), ("K", 0.75), ("IH", 0.63), ("P", 0.40)]),
    (&[("M", 1.00), ("OY", 0.86), ("OY", 0.81), ("AA", 0.47), ("K", 0.87), ("K", 0.91), ("IH", 0.67)], &[("M", 0.40), ("IH", 0.51), ("T", 0.93), ("IH", 0.68), ("IH", 0.53), ("T", 0.84), ("AH", 0.48), ("EH", 0.53)]),
    (&[("DH", 0.34), ("IH", 0.64), ("T", 0.93), ("IH", 0.77), ("IH", 0.47), ("T", 0.88), ("AH", 0.78)], &[("K", 0.99), ("K", 0.74), ("SH", 0.50), ("N", 0.49), ("W", 1.00), ("DH", 0.82), ("M", 1.00), ("T", 0.34)]),
    (&[("K", 0.97), ("K", 0.67), ("SH", 0.67), ("N", 0.55), ("W", 1.00), ("DH", 0.90), ("M", 1.00)], &[("L", 0.62), ("T", 0.71), ("IH", 0.55), ("AH", 0.86), ("L", 0.97), ("W", 1.00), ("ER", 0.97), ("ER", 0.54)]),
    (&[("L", 0.48), ("T", 0.65), ("IH", 0.63), ("AH", 0.83), ("L", 0.97), ("W", 1.00), ("ER", 0.92)], &[("ER", 0.59), ("AH", 0.32), ("Z", 0.58), ("sil", 0.35), ("AH", 0.50), ("D", 0.42), ("D", 0.72), ("AA", 0.14)]),
    (&[("ER", 0.64), ("AH", 0.39), ("Z", 0.50), ("sil", 0.21), ("AH", 0.50), ("N", 0.53), ("D", 0.49)], &[("P", 0.23), ("R", 0.74), ("EY", 0.98), ("NG", 0.22), ("EH", 0.41), ("IH", 0.44), ("Z", 0.72), ("sil", 0.41)]),
    (&[("P", 0.18), ("R", 0.74), ("EY", 0.98), ("NG", 0.25), ("AH", 0.35), ("IH", 0.74), ("Z", 0.62), ("N", 0.50), ("sil", 0.74), ("sil", 0.57)], &[("sil", 0.49)]),
];

/// Seconds of audio per live model step, and per encoder frame.
const STEP_SECS: f64 = 0.56;
const FRAME_SECS: f32 = 0.08;

/// What the microphone hears in a stretch of a moment.
#[derive(Clone, Copy, PartialEq)]
enum Sound {
    /// Syllables at about 6.6 a second, phrase swells, pitch moving.
    Speech,
    /// Room tone.
    Quiet,
    /// Broadband noise with no voice in it: a fan, a keyboard.
    Noise,
}

/// A moment being written: the messages so far and the state the runtime
/// files are in.
struct Take {
    inputs: Vec<(f64, Input<Msg>)>,
    files: Files,
    doc: LiveDoc,
    /// Scene time when the recording started: audio time 0.
    rec_start: f64,
    /// Audio frames sent, for their sequence numbers and the sample clock.
    seq: u32,
    /// Timing of the words in the current hypothesis, in audio seconds.
    timed: Vec<TimedWord>,
    /// `preview-signals.json` so far.
    signals: voxtype::osd::live::ModelSignals,
    /// Words shown at low confidence while they are the newest.
    unsure: Vec<&'static str>,
}

impl Take {
    fn new(rec_start: f64) -> Take {
        Take {
            inputs: Vec::new(),
            files: Files::default(),
            doc: LiveDoc::default(),
            rec_start,
            seq: 0,
            timed: Vec::new(),
            signals: Default::default(),
            unsure: vec!["popup", "swish", "confi"],
        }
    }

    /// The daemon's edit block at `t`: `phase` with the text so far frozen,
    /// or None when the edit is over.
    fn edit(&mut self, t: f64, phase: Option<&str>) {
        self.doc.edit = phase.map(|phase| voxtype::osd::edit_ipc::EditBlock {
            session: 1,
            phase: phase.into(),
            frozen: self.files.preview.clone(),
            tail: String::new(),
            pause: 1,
            raw_words: self.files.preview.split_whitespace().count(),
        });
        self.files.live = Some(self.doc.clone());
        self.files_at(t);
    }

    fn files_at(&mut self, t: f64) {
        self.inputs
            .push((t, Input::Msg(Msg::Files(self.files.clone()))));
    }

    /// The recording starts at `t` (the state file says so).
    fn record(&mut self, t: f64) {
        self.files.recording = true;
        self.files_at(t);
    }

    /// What the microphone hears from `from` to `to`: two 10 ms frames and
    /// their voice features every 20 ms.
    fn sound(&mut self, from: f64, to: f64, sound: Sound) {
        let tau = std::f64::consts::TAU;
        let mut t = from;
        while t < to - 1e-9 {
            let mut frames = Vec::new();
            let mut voice = Vec::new();
            for k in 0..2 {
                let ft = t + k as f64 * 0.01;
                let a = ft - self.rec_start;
                let (amp, voiced, onset, f0, flat) = match sound {
                    Sound::Speech => {
                        // 6.6 syllables a second; an onset where each starts.
                        let syl = (a * 3.3 * tau).sin().abs();
                        let phase = (a * 6.6).fract();
                        let phrase = 0.6 + 0.4 * (a * 0.45 * tau).sin();
                        // The pitch rises over the last 0.4 s before a pause.
                        let tail = ((ft - (to - 0.4)) / 0.4).clamp(0.0, 1.0);
                        let f0 = 135.0 + 18.0 * (a * 0.7 * tau).sin() + 45.0 * tail;
                        let amp = 0.004 + 0.28 * syl * phrase;
                        let onset = if phase < 0.066 { 0.85 } else { 0.05 };
                        (amp, amp > 0.05, onset, f0, 0.12)
                    }
                    Sound::Quiet => (0.002, false, 0.0, 0.0, 0.7),
                    Sound::Noise => {
                        let wobble = 0.5 + 0.5 * (a * 11.0).sin().abs();
                        (0.02 + 0.03 * wobble, false, 0.1, 0.0, 0.85)
                    }
                };
                let amp = amp as f32;
                let rms_dbfs = 20.0 * (amp * 0.7).max(1e-6).log10();
                let peak_dbfs = 20.0 * amp.max(1e-6).log10();
                frames.push(AudioFrame {
                    seq: self.seq,
                    min: -amp,
                    max: amp,
                    peak_dbfs,
                });
                let mut mel = [0u8; voxtype::audio::features::MEL_BANDS];
                for (i, m) in mel.iter_mut().enumerate() {
                    // Voice falls off above the low bands; noise is flat.
                    let tilt = if voiced {
                        i as f32 * 2.2
                    } else {
                        i as f32 * 0.4
                    };
                    let db = rms_dbfs - tilt;
                    *m = ((db + 100.0) / 100.0 * 255.0).clamp(0.0, 255.0) as u8;
                }
                self.seq += 1;
                voice.push(Voice {
                    seq: self.seq - 1,
                    features: AudioFeatures {
                        rms_dbfs,
                        f0_hz: if voiced { f0 as f32 } else { 0.0 },
                        voiced,
                        centroid_hz: if amp > 0.01 {
                            900.0 + 600.0 * flat as f32
                        } else {
                            0.0
                        },
                        flatness: flat as f32,
                        mel,
                        onset: onset as f32,
                    },
                    peak_dbfs,
                    clock: self.seq as u64 * (WIRE_SAMPLE_RATE as u64 / FRAME_HZ as u64),
                });
            }
            self.inputs.push((t, Input::Msg(Msg::Audio(frames, voice))));
            t += 0.02;
        }
    }

    /// The daemon's silence pump after the mic stops, from `from` to `to`.
    fn pump(&mut self, from: f64, to: f64) {
        let mut pump = 0u32;
        let mut t = from;
        while t < to {
            let silent = AudioFrame {
                seq: pump,
                min: 0.0,
                max: 0.0,
                peak_dbfs: -120.0,
            };
            self.inputs
                .push((t, Input::Msg(Msg::Audio(vec![silent], Vec::new()))));
            pump += 1;
            t += 0.033;
        }
    }

    /// The live text becomes `text` at `t`. Words that stay keep their
    /// timing; new ones are timed as just said.
    fn say(&mut self, t: f64, text: &str) {
        let now = (t - self.rec_start) as f32;
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut timed = Vec::with_capacity(words.len());
        let mut prev_end = 0.0f32;
        for (i, w) in words.iter().enumerate() {
            let kept = self.timed.get(i).filter(|o| o.text == *w);
            let (start, end) = match kept {
                Some(o) => (o.start, o.end),
                None => {
                    let start = prev_end.max(now - 0.35).max(0.0);
                    (start, (start + 0.25).min(now.max(start + 0.05)))
                }
            };
            prev_end = end;
            let newest = i + 1 == words.len();
            let unsure = self.unsure.contains(w) && (newest || *w == "popup");
            timed.push(TimedWord {
                text: w.to_string(),
                start,
                end,
                conf: Some(if unsure { 0.4 } else { 0.93 }),
            });
        }
        self.doc.apply_preview(text, Some(&timed));
        self.timed = timed;
        self.files.preview = text.trim().to_string();
        self.files.live = Some(self.doc.clone());
        self.files_at(t);
    }

    /// The live text at `t` is the first `n` committed tokens of the lookahead
    /// moment, with their own timing, confidence and detail.
    fn say_tokens(&mut self, t: f64, n: usize) {
        use voxtype::osd::live::WordDetail;
        let words = voxtype::transcribe::revisions::words_with_details(
            LOOKAHEAD_TOKENS[..n].iter().map(|&(text, frame, p, entropy, margin, runner_up)| {
                let start = frame as f32 * FRAME_SECS;
                let detail = WordDetail {
                    entropy,
                    margin,
                    runner_up: runner_up.map(str::to_string),
                };
                (text, start, start + FRAME_SECS, Some(p), Some(detail))
            }),
        );
        let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let timed: Vec<TimedWord> = words
            .iter()
            .map(|w| TimedWord {
                text: w.text.clone(),
                start: w.start,
                end: w.end,
                conf: w.conf,
            })
            .collect();
        let details: Vec<_> = words.iter().map(|w| w.detail.clone()).collect();
        self.doc.apply_preview_with_details(&text, Some(&timed), Some(&details));
        self.timed = timed;
        self.files.preview = text;
        self.files.live = Some(self.doc.clone());
        self.files_at(t);
    }

    /// One step of the live model at `t`: its new frames and its tail, as
    /// `preview-signals.json` gets them.
    fn signals_step(
        &mut self,
        t: f64,
        tail: &[(&str, u32, f32)],
        frames: &[LookaheadFrame],
        sounds: Option<&LookaheadSounds>,
    ) {
        use voxtype::osd::live::{FrameLevel, Guess, SignalsStep, SoundFrame, TailToken};
        let audio_s = (t - self.rec_start) as f32;
        let first = self.signals.frames_total as usize;
        let levels = frames
            .iter()
            .enumerate()
            .map(|(i, &(level, top))| FrameLevel {
                t: (first + i) as f32 * FRAME_SECS,
                level,
                top: top
                    .map(|(text, logp)| Guess {
                        text: text.into(),
                        logp,
                    })
                    .into_iter()
                    .collect(),
            })
            .collect();
        let end = (first + frames.len()) as u32;
        let tail: Vec<TailToken> = tail
            .iter()
            .map(|&(text, edge, conf)| {
                let frame = (end + 7).saturating_sub(edge + 1);
                TailToken {
                    text: text.into(),
                    start: frame as f32 * FRAME_SECS,
                    end: (frame + 1) as f32 * FRAME_SECS,
                    edge,
                    conf,
                    detail: None,
                }
            })
            .collect();
        let mut with_tail = self.files.preview.clone();
        for tk in &tail {
            with_tail.push_str(&tk.text);
        }
        // The sounds go on from the last committed one.
        let next = self
            .signals
            .sounds
            .last()
            .map_or(0, |f| (f.t / FRAME_SECS).round() as usize + 1);
        let sound_frames = |first: usize, phones: &[(&str, f32)]| -> Vec<SoundFrame> {
            phones
                .iter()
                .enumerate()
                .map(|(i, &(text, p))| SoundFrame {
                    t: (first + i) as f32 * FRAME_SECS,
                    top: vec![Guess {
                        text: text.into(),
                        logp: p.ln(),
                    }],
                })
                .collect()
        };
        let (sounds, sound_tail) = match sounds {
            Some((firm, ahead)) => (
                sound_frames(next, firm),
                sound_frames(next + firm.len(), ahead),
            ),
            None => Default::default(),
        };
        self.signals.apply(SignalsStep {
            step: self.signals.step + 1,
            audio_s,
            frames: levels,
            tail,
            with_tail: with_tail.trim().to_string(),
            sounds,
            sound_tail,
        });
        self.inputs.push((
            t,
            Input::Msg(Msg::ModelSignals(Box::new(self.signals.clone()))),
        ));
    }

    /// Speech from `t` saying `words` one after another, `per` seconds
    /// apart, each added to the text so far. Returns when the last ends.
    fn speak(&mut self, t: f64, words: &[&str], per: f64) -> f64 {
        let end = t + 0.35 + words.len() as f64 * per;
        self.sound(t, end, Sound::Speech);
        let mut so_far: Vec<String> = self
            .files
            .preview
            .split_whitespace()
            .map(str::to_string)
            .collect();
        for (i, w) in words.iter().enumerate() {
            so_far.push(w.to_string());
            self.say(t + 0.35 + i as f64 * per, &so_far.join(" "));
        }
        end
    }

    /// The spoken correction the daemon reports at `t`, None when there is
    /// none to show.
    fn correction(&mut self, t: f64, view: Option<voxtype::osd::correction::CorrectionView>) {
        self.doc.correction = view;
        self.files.live = Some(self.doc.clone());
        self.files_at(t);
    }

    /// A model load with real progress from `t` for `secs`.
    fn load(&mut self, t: f64, secs: f64) {
        let steps = (secs / 0.25).ceil() as usize;
        self.files.finishing = true;
        for k in 0..=steps {
            let f = k as f64 / steps as f64;
            self.files.fraction = Some(f);
            self.files.status = format!("Loading: live text model, {} s", (k as f64 * 0.25) as u32);
            self.files_at(t + k as f64 * secs / steps as f64);
        }
        self.files.finishing = false;
        self.files.fraction = None;
        self.files.status.clear();
        self.files_at(t + secs + 0.01);
    }

    /// The recording stops at `t`.
    fn stop(&mut self, t: f64) {
        self.files.recording = false;
        self.files.transcribing = true;
        self.files_at(t);
    }

    /// The final pass from `t` for `secs`, with the finishing fraction
    /// moving, then `text` as the final text. Returns when it is shown.
    fn finish(&mut self, t: f64, secs: f64, text: &str) -> f64 {
        self.files.finishing = true;
        let steps = 6;
        for k in 0..steps {
            let f = (k + 1) as f64 / steps as f64;
            self.files.fraction = Some(f);
            let left = ((1.0 - f) * secs * 2.0).ceil() as u32;
            self.files.status = format!("Finishing: {left} s of audio left");
            self.files_at(t + 0.05 + k as f64 * secs / steps as f64);
        }
        let shown = t + secs + 0.1;
        self.files.preview = text.to_string();
        self.files_at(shown);
        shown
    }

    /// How the dictation ended, at `t`: the daemon goes idle and says so.
    fn end(&mut self, t: f64, outcome: Outcome) {
        self.files.recording = false;
        self.files.transcribing = false;
        self.files.finishing = false;
        self.files.fraction = None;
        self.files.status.clear();
        self.files.outcome = Some(outcome);
        self.files_at(t);
    }

    fn scenario(mut self, look: &Look, duration: f64) -> vo::Scenario<Msg> {
        self.inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
        vo::Scenario {
            size: look.surface(),
            scale: 1.0,
            duration,
            inputs: self.inputs,
        }
    }
}

/// How the final text was delivered, for the delivery moments.
#[derive(Clone, Copy)]
enum Delivery {
    Typed,
    PastedEnter,
}

/// The delivery at `t`: `result.json` says how the final text went out,
/// and the daemon goes idle.
fn deliver(take: &mut Take, t: f64, how: Delivery) {
    let text = take.files.preview.clone();
    let chars = text.chars().count();
    let (method, enter, output_ms) = match how {
        Delivery::Typed => ("wtype", false, chars as u64 * 3),
        Delivery::PastedEnter => ("paste", true, 40),
    };
    take.files.delivery = Some(voxtype::osd::outcome::Delivery {
        text,
        chars,
        method: method.into(),
        enter,
        output_ms,
        latency_ms: Some(620),
        audio_s: Some((t - take.rec_start) as f32),
    });
    take.end(t, Outcome::Done);
}

/// The final text the final pass makes of `words`: a capital, the commas and
/// full stops, and the words the live text got wrong put right.
fn final_text(words: &[&str]) -> String {
    let mut s = words.join(" ");
    s = s
        .replace("swish", "swiss")
        .replace("pop up", "popup")
        .replace("wave form", "waveform")
        .replace(" so the", ", so the")
        .replace(" then press", ". Then press");
    let first = s[..1].to_uppercase();
    s.replace_range(0..1, &first);
    s.push('.');
    s
}

/// The scripted dictation for `moment`, as `look` would get it.
pub(crate) fn scenario(look: &Look, moment: Moment) -> vo::Scenario<Msg> {
    let s0 = 0.2;
    let mut k = Take::new(s0);
    let per = 0.32;
    match moment {
        Moment::Dictation => super::scenario(look, false),
        Moment::Cancel => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 9), per);
            k.sound(t, t + 0.3, Sound::Speech);
            k.end(t + 0.3, Outcome::Cancelled);
            k.scenario(look, t + 1.6)
        }
        Moment::FirstWord => {
            k.record(s0);
            // Voice for 1.5 s before the engine gives the first word.
            k.sound(s0, s0 + 1.5, Sound::Speech);
            let t = k.speak(s0 + 1.5, &sentence_words(0, 4), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 4)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Pause => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 6), per);
            k.sound(t, t + 1.6, Sound::Quiet);
            let t = k.speak(t + 1.6, &sentence_words(6, 6), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 12)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::LongSilence => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 3), per);
            k.sound(t, t + LONG_SILENCE, Sound::Quiet);
            let t = k.speak(t + LONG_SILENCE, &sentence_words(3, 5), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 8)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::RevisionBurst => {
            k.record(s0);
            let mut t = k.speak(
                s0,
                &["oh", "pen", "the", "confirmation", "file", "and", "set"],
                per,
            );
            // A burst: the newest words rewritten six times in about a second
            // and a half, two words becoming one twice and a word on the older
            // text put right. Each merge stays on one line, so its hinge
            // (MO-21) runs even in a narrow look where the next word wraps.
            let burst = [
                "open the confirmation file and set the pop",
                "open the confirmation file and set the pop up",
                "open the confirmation file and set the popup",
                "open the confirmation file and set the popup theme",
                "open the configuration file and set the popup theme to swish",
                "open the configuration file and set the popup theme to swiss",
            ];
            let burst_end = t + 0.22 * burst.len() as f64 + 0.2;
            k.sound(t, burst_end, Sound::Speech);
            for b in burst {
                t += 0.22;
                k.say(t, b);
            }
            let t = k.speak(burst_end, &sentence_words(11, 5), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 16)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Lattice => {
            k.record(s0);
            let mut t = s0;
            let mut cursor = s0;
            let mut said: Vec<&str> = Vec::new();
            for (i, (word, pause)) in LATTICE_SCRIPT.iter().enumerate() {
                // A word is timed 0.35 s before it is said and lasts 0.25 s,
                // so the pause the popup measures is the gap less 0.25 s.
                t += if i == 0 { 0.35 } else { 0.25 + pause };
                let from = t - 0.33;
                if from > cursor {
                    k.sound(cursor, from, Sound::Quiet);
                }
                k.sound(from.max(cursor), t - 0.01, Sound::Speech);
                cursor = t - 0.01;
                said.push(word);
                k.say(t, &said.join(" "));
                if *word == "swish." {
                    // The engine hears it again and puts it right.
                    k.sound(cursor, t + 0.5, Sound::Quiet);
                    cursor = t + 0.5;
                    *said.last_mut().unwrap() = "swiss.";
                    k.say(t + 0.5, &said.join(" "));
                }
            }
            let end = cursor.max(t) + 0.05;
            k.stop(end);
            let text = said.join(" ");
            let shown = k.finish(end, 0.6, &text);
            k.pump(end + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Wrap => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 16), 0.26);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 16)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Overflow => {
            k.record(s0);
            let all = sentence_words(0, 60);
            let t = k.speak(s0, &all, 0.18);
            k.stop(t);
            let shown = k.finish(t, 0.8, &final_text(&all));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.3, Outcome::Done);
            k.scenario(look, shown + 1.5)
        }
        Moment::FinalPass | Moment::Typed | Moment::PastedEnter => {
            if moment == Moment::Typed {
                // The daemon names the focused app during the dictation, so the receipt
                // can say where the text went and show the app's icon (EG-24).
                k.doc.target = Some(voxtype::osd::live::LiveTarget {
                    app: "org.kde.konsole".into(),
                });
            }
            k.record(s0);
            // The live text keeps three mistakes for the final pass to fix.
            let words = [
                "set", "the", "pop", "up", "theme", "to", "swish", "so", "the", "wave", "form",
                "shows", "above", "the", "words",
            ];
            let t = k.speak(s0, &words, 0.24);
            k.stop(t);
            let fin = final_text(&words);
            let secs = if moment == Moment::FinalPass {
                2.4
            } else {
                1.0
            };
            let shown = k.finish(t, secs, &fin);
            k.pump(t + 0.03, shown + 0.4);
            match moment {
                Moment::Typed => deliver(&mut k, shown + 0.4, Delivery::Typed),
                Moment::PastedEnter => deliver(&mut k, shown + 0.4, Delivery::PastedEnter),
                _ => k.end(shown + 0.4, Outcome::Done),
            }
            k.scenario(look, shown + 1.8)
        }
        Moment::NoSpeech => {
            k.record(s0);
            k.sound(s0, s0 + 2.4, Sound::Noise);
            k.stop(s0 + 2.4);
            k.pump(s0 + 2.43, s0 + 2.9);
            k.end(s0 + 2.9, Outcome::Empty);
            k.scenario(look, s0 + 4.0)
        }
        Moment::Failure => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 7), per);
            k.stop(t);
            k.files.finishing = true;
            k.files.fraction = Some(0.3);
            k.files.status = "Finishing: 2 s of audio left".into();
            k.files_at(t + 0.1);
            k.pump(t + 0.03, t + 0.9);
            k.end(t + 0.9, Outcome::Failed);
            k.scenario(look, t + 2.2)
        }
        Moment::Loading => {
            k.record(s0);
            k.sound(s0, s0 + 3.0, Sound::Quiet);
            k.load(s0, 3.0);
            let t = k.speak(s0 + 3.1, &sentence_words(0, 6), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 6)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Lookahead => {
            k.record(s0);
            let n = LOOKAHEAD_STEPS.len();
            // A step runs once its chunk and the right context are in.
            let step_at = |i: usize| s0 + (i + 2) as f64 * STEP_SECS;
            let speech_end = s0 + 4.75;
            k.sound(s0, speech_end, Sound::Speech);
            let mut t = s0;
            let mut said = 0;
            for (i, (committed, tail, frames)) in LOOKAHEAD_STEPS.iter().enumerate() {
                // The flush comes when the recording stops.
                t = if i + 1 == n { speech_end + 0.05 } else { step_at(i) };
                if i + 1 == n {
                    k.stop(t - 0.01);
                }
                if *committed != said {
                    k.say_tokens(t, *committed);
                    said = *committed;
                }
                k.signals_step(t, tail, frames, LOOKAHEAD_SOUNDS.get(i));
            }
            let text = k.files.preview.clone();
            let shown = k.finish(t, 0.6, &text);
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::LookSwitch => {
            k.record(s0);
            let t = k.speak(s0, &sentence_words(0, 6), per);
            // The chord steps to the next look mid-dictation, as the
            // daemon's step file says.
            k.files.look_step = Some(look_step::Step {
                stamp: 1,
                forward: true,
            });
            k.files_at(t + 0.1);
            let t = k.speak(t + 0.2, &sentence_words(6, 6), per);
            k.stop(t);
            let shown = k.finish(t, 0.6, &final_text(&sentence_words(0, 12)));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Correction => {
            use voxtype::osd::correction::{CorrectionNote, CorrectionView};
            let hold = |hearing: &str| CorrectionView {
                hearing: Some(hearing.into()),
                pending: false,
                spelling: None,
                note: None,
            };
            let note = |id, from: &str, to: &str, dropped: Option<&str>| CorrectionView {
                hearing: None,
                pending: false,
                spelling: None,
                note: Some(CorrectionNote {
                    id,
                    from: from.into(),
                    to: to.into(),
                    dropped: dropped.map(str::to_string),
                    learned: false,
                }),
            };
            k.record(s0);
            let t = k.speak(s0, &["set", "the", "pop", "up", "theme", "to", "swiss"], per);
            // Held from t: "popup" said again, placed on "pop up".
            k.correction(t + 0.45, Some(hold("")));
            k.sound(t, t + 1.0, Sound::Speech);
            k.correction(t + 0.8, Some(hold("popup")));
            let pending = CorrectionView {
                hearing: None,
                pending: true,
                spelling: None,
                note: None,
            };
            k.correction(t + 1.0, Some(pending));
            k.say(t + 1.4, "set the popup theme to swiss");
            k.correction(t + 1.4, Some(note(1, "pop up", "popup", None)));
            let t = k.speak(t + 1.6, &sentence_words(11, 4), per);
            // A second hold that sounds like nothing on screen.
            k.correction(t + 0.45, Some(hold("")));
            k.sound(t, t + 1.0, Sound::Speech);
            k.correction(t + 0.8, Some(hold("configuration")));
            k.correction(t + 1.4, Some(note(2, "", "configuration", Some("no match"))));
            let t = t + 1.4 + super::correction_note::NOTE_SECS;
            k.stop(t);
            let words: Vec<&str> = k.files.preview.split_whitespace().collect();
            let text = final_text(&words);
            let shown = k.finish(t, 0.6, &text);
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Spell => {
            use voxtype::osd::correction::{CorrectionNote, CorrectionView};
            use voxtype::text::spell;
            let hold = |hearing: &str| {
                let words: Vec<&str> = hearing.split_whitespace().collect();
                CorrectionView {
                    hearing: Some(hearing.into()),
                    spelling: spell::letters(&words).map(|(letters, _)| letters),
                    ..CorrectionView::default()
                }
            };
            k.record(s0);
            let t = k.speak(s0, &["ask", "the", "glim", "bot", "for", "the", "logs"], per);
            // Held from t: "spell capital golf lima india mike bravo oscar tango".
            k.correction(t + 0.45, Some(hold("")));
            k.sound(t, t + 3.2, Sound::Speech);
            let mut said = String::new();
            for (i, word) in ["Spell", "capital", "golf", "lima", "india", "mike", "bravo", "oscar", "tango"]
                .iter()
                .enumerate()
            {
                said = format!("{said} {word}").trim().to_string();
                k.correction(t + 0.6 + 0.3 * i as f64, Some(hold(&said)));
            }
            let pending = CorrectionView {
                pending: true,
                ..CorrectionView::default()
            };
            k.correction(t + 3.4, Some(pending));
            k.say(t + 3.8, "ask the Glimbot for the logs");
            let note = CorrectionView {
                note: Some(CorrectionNote {
                    id: 1,
                    from: "glim bot".into(),
                    to: "Glimbot".into(),
                    dropped: None,
                    learned: true,
                }),
                ..CorrectionView::default()
            };
            k.correction(t + 3.8, Some(note));
            let t = t + 3.8 + super::correction_note::NOTE_SECS;
            k.stop(t);
            let words: Vec<&str> = k.files.preview.split_whitespace().collect();
            let text = final_text(&words);
            let shown = k.finish(t, 0.6, &text);
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
        Moment::Edit => edit_scenario(look, 2),
        Moment::TimeLimit => {
            // Runs to the limit: the recording stops itself. `Files` does not
            // carry the limit yet; MO-35 adds it.
            k.record(s0);
            let all = sentence_words(0, 40);
            let t = k.speak(s0, &all, 0.3);
            k.stop(t);
            let shown = k.finish(t, 0.8, &final_text(&all));
            k.pump(t + 0.03, shown);
            k.end(shown + 0.2, Outcome::Done);
            k.scenario(look, shown + 1.4)
        }
    }
}

/// A keyboard edit as `look` would get it (plan M7.6): 26 words dictated,
/// a tap in the pause, the caret `ups` lines up from the end, " quickly"
/// typed, Enter, and the daemon releasing and delivering. The `edit`
/// moment is `ups = 2`. Run it on [`popup_for`]`(Moment::Edit, ..)`.
pub(crate) fn edit_scenario(look: &Look, ups: usize) -> vo::Scenario<Msg> {
    use vo::scene::input::{LogicalKey, Mods, NamedKey};
    let s0 = 0.2;
    let mut k = Take::new(s0);
    k.record(s0);
    let t = k.speak(s0, &sentence_words(0, 26), 0.32);
    // A tap in the pause: pending, then the text is the user's.
    let up = t + 0.7;
    let typed = up + 0.25 * ups as f64 + 0.35;
    let (enter, done) = (typed + 0.9, typed + 1.15);
    k.sound(t, done + 1.6, Sound::Quiet);
    k.edit(t + 0.1, Some("pending"));
    k.edit(t + 0.35, Some("editing"));
    k.inputs.push((
        t + 0.4,
        Input::KeyboardFocus {
            gained: true,
            requested: true,
        },
    ));
    // The daemon takes the edited text from the buffer, as the popup's
    // keys leave it, and publishes and delivers that.
    let edited = edited_text(look, &k.files.preview, ups, " quickly");
    k.say(done, &edited);
    k.edit(done, Some("releasing"));
    k.edit(done + 0.2, None);
    deliver(&mut k, done + 0.2, Delivery::PastedEnter);
    let mut s = k.scenario(look, done + 1.6);
    // Keys well apart, so the daemon's answers fall after them.
    for i in 0..ups {
        s.key(
            up + 0.25 * i as f64,
            LogicalKey::Named(NamedKey::Up),
            None,
            Mods::NONE,
        );
    }
    s.type_text(typed, " quickly");
    s.key(enter, LogicalKey::Named(NamedKey::Enter), None, Mods::NONE);
    s
}

/// `text` after the edit moment's keys in `look`: the caret from the end
/// `ups` lines up by the look's own line breaks, then `typed`.
fn edited_text(look: &Look, text: &str, ups: usize, typed: &str) -> String {
    use vo::scene::text::TextLayout;
    use vo_core::edit::{EditAction, EditBuffer};
    let text_box = super::edit_site::site(look.theme).text_box(look);
    let measure = vo::ShapedLayout::new(look.fonts.clone());
    let lines = measure.layout(
        text,
        &super::edit::edit_style(text_box.style),
        Some(text_box.width),
    );
    let mut buffer = EditBuffer::new(text);
    for _ in 0..ups {
        let up = EditAction::MoveLine {
            down: false,
            extend: false,
        };
        buffer.apply(up, &lines, 0.0);
    }
    buffer.apply_plain(EditAction::Insert(typed.into()), 0.0);
    buffer.text().to_string()
}

/// `popup` set up for `moment`: in the edit moment it has a keyboard edit
/// link that reads and writes no files.
pub(crate) fn popup_for(moment: Moment, mut popup: Popup) -> Popup {
    if moment == Moment::Edit {
        popup.edit = Some(super::edit_link::EditLink::offline());
    }
    popup
}

/// `--offline DIR --moment NAME`: renders one moment to PNG frames.
pub(crate) fn offline(
    look: Look,
    fonts: FontRegistry,
    dir: &Path,
    fps: f64,
    name: &str,
    gpu_times: bool,
) -> anyhow::Result<()> {
    let m = parse_or_list(name)?;
    let s = scenario(&look, m);
    let (times, sink) = gpu_time_sink(gpu_times);
    let options = Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        frame_timings: sink,
        renderer_setup: Some(Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let n = vo::render_offline_with(
        popup_for(m, Popup::scripted(look, &fonts)),
        &s,
        fps,
        dir,
        options,
    )
    .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
    eprintln!(
        "voxtype-osd-vo: wrote {n} frames of {} to {}",
        m.name(),
        dir.display()
    );
    if gpu_times {
        print_gpu_times(&times.lock().unwrap());
    }
    Ok(())
}

fn parse_or_list(name: &str) -> anyhow::Result<Moment> {
    Moment::parse(name).ok_or_else(|| {
        anyhow::anyhow!("no moment {name:?}; the moments are {}", Moment::names())
    })
}

/// `--live-moment NAME`: the popup plays one moment on its real surface.
/// The moment's messages come as timers in scene time, so a frame's `t` in
/// the diagnostics is the moment's own time. At the start it prints
/// `live-moment: <name> starts at uptime <s>` (from /proc/uptime), so a
/// script can sample the process's CPU time at moment times.
pub(crate) fn live(
    look: Look,
    fonts: FontRegistry,
    cfg: &OsdConfig,
    name: &str,
) -> anyhow::Result<()> {
    let m = parse_or_list(name)?;
    let inputs = scenario(&look, m)
        .inputs
        .into_iter()
        .filter_map(|(t, input)| match input {
            Input::Msg(msg) => Some((t, msg)),
            _ => None,
        })
        .collect();
    let options = Options {
        fonts: fonts.clone(),
        cpu_first_frame: true,
        diagnostics: Some(Box::new(LogDiagnostics::new(Some(heap)))),
        default_motion: default_motion(&look),
        renderer_setup: Some(Arc::new(vo_3d::install)),
        margin: surface_margin(&look),
        ..Options::default()
    };
    let spec = live_spec(cfg, &look);
    // Warm up as the daemon does at start, so the moment shows what a
    // dictation after the first shows: no pipeline built mid-moment.
    let mut resident = vo::Resident::new();
    let warm = Options::<Msg> {
        fonts: fonts.clone(),
        renderer_setup: Some(Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let (warm_look, warm_fonts) = (look.clone(), fonts.clone());
    resident
        .warm_up(&warm, move || warm_frames(warm_look, warm_fonts))
        .map_err(|e| anyhow::anyhow!("live moment: {e}"))?;
    // The moment's times start with the show, so start it once warm.
    let waited = std::time::Instant::now();
    while !resident.warm_up_done() && waited.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let played = Played {
        name: m.name(),
        popup: Popup {
            live_surface: true,
            ..Popup::scripted(look, &fonts)
        },
        inputs,
    };
    let _boost = crate::realtime::raise(cfg.realtime);
    resident
        .run(played, spec, options)
        .map_err(|e| anyhow::anyhow!("live moment: {e}"))
}

/// A popup fed one moment's messages by timers (`--live-moment`).
struct Played {
    name: &'static str,
    popup: Popup,
    inputs: Vec<(f64, Msg)>,
}

impl App for Played {
    type Msg = Msg;

    fn init(&mut self, cx: &mut Cx<Msg>) {
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0);
        eprintln!(
            "live-moment: {} starts at uptime {uptime:.3} (scene t={:.3})",
            self.name,
            cx.t()
        );
        let t0 = cx.t();
        for (t, msg) in self.inputs.drain(..) {
            cx.after((t - t0).max(0.0), msg);
        }
        self.popup.init(cx);
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        self.popup.update(msg, cx);
    }

    fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
        self.popup.view(tokens)
    }

    fn surfaces(&self, tokens: &vo::Tokens) -> Vec<vo::SurfaceView<Msg>> {
        self.popup.surfaces(tokens)
    }

    fn main_surface(&self) -> Option<(Size, Placement)> {
        self.popup.main_surface()
    }

    fn frame_pace(&self) -> Option<f64> {
        self.popup.frame_pace()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_look() -> Look {
        let cfg = OsdConfig::default();
        let theme = PlasmaTheme::default();
        let fonts = load_fonts(&theme).unwrap();
        Look::new(&cfg, &theme, String::new(), &fonts)
    }

    fn files(s: &vo::Scenario<Msg>) -> Vec<(f64, Files)> {
        s.inputs
            .iter()
            .filter_map(|(t, i)| match i {
                Input::Msg(Msg::Files(f)) => Some((*t, f.clone())),
                _ => None,
            })
            .collect()
    }

    /// The popup, reporting its look after every message.
    struct Spy(Popup, std::rc::Rc<std::cell::RefCell<Vec<OsdTheme>>>);

    impl vo::App for Spy {
        type Msg = Msg;

        fn init(&mut self, cx: &mut Cx<Msg>) {
            self.0.init(cx);
        }

        fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
            self.0.update(msg, cx);
            let mut seen = self.1.borrow_mut();
            if seen.last() != Some(&self.0.look.theme) {
                seen.push(self.0.look.theme);
            }
        }

        fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
            self.0.view(tokens)
        }

        fn surfaces(&self, tokens: &vo::Tokens) -> Vec<vo::SurfaceView<Msg>> {
            self.0.surfaces(tokens)
        }

        fn main_surface(&self) -> Option<(Size, vo::Placement)> {
            self.0.main_surface()
        }
    }

    #[test]
    fn the_look_switch_moment_steps_the_look_and_saves_nothing() {
        // The saved look lives under XDG_STATE_HOME: point it at an empty
        // folder, and nothing may appear there.
        let state = std::path::PathBuf::from(format!(
            "/var/tmp/voxtype-test-look-switch-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        std::env::set_var("XDG_STATE_HOME", &state);

        let mut look = test_look();
        look.theme = OsdTheme::Swiss;
        let fonts = load_fonts(&PlasmaTheme::default()).unwrap();
        let s = scenario(&look, Moment::LookSwitch);
        let seen = std::rc::Rc::new(std::cell::RefCell::new(vec![OsdTheme::Swiss]));
        let spy = Spy(Popup::scripted(look, &fonts), seen.clone());
        let report = vo::exit_check(spy, &s, 10.0, Options::default());
        let next = look_step::step(&OsdTheme::VO_LOOKS, OsdTheme::Swiss, true);
        assert_eq!(
            *seen.borrow(),
            vec![OsdTheme::Swiss, next],
            "one step, to the next look"
        );
        assert!(report.ending.after_exit().is_some(), "{report:?}");
        let left: Vec<_> = walk(&state);
        let _ = std::fs::remove_dir_all(&state);
        assert!(left.is_empty(), "the scripted step saved {left:?}");
    }

    fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn names_round_trip() {
        for m in Moment::ALL {
            assert_eq!(Moment::parse(m.name()), Some(*m));
        }
        assert_eq!(Moment::parse("Revision_Burst"), Some(Moment::RevisionBurst));
        assert_eq!(Moment::parse("nope"), None);
    }

    /// Each message is one `update` and one view rebuild, so room tone must come as one
    /// message per capture batch: its frames and their voice features together.
    #[test]
    fn a_room_tone_batch_is_one_message() {
        let look = test_look();
        let s = scenario(&look, Moment::LongSilence);
        let mut at: Vec<f64> = Vec::new();
        for (t, input) in &s.inputs {
            if let Input::Msg(Msg::Audio(frames, voice)) = input {
                assert!(!frames.is_empty());
                if frames.iter().all(|f| !f.is_pump_silence()) {
                    assert_eq!(voice.len(), frames.len(), "features ride with their frames");
                }
                at.push(*t);
            }
        }
        assert!(at.len() > 100, "the moment records room tone");
        assert!(
            at.windows(2).all(|w| w[1] > w[0]),
            "two audio messages for one batch"
        );
    }

    /// LT-12: the long silence outlasts the lattice's quiet motion by more
    /// than a second, so a render sees it come to rest.
    #[test]
    fn the_long_silence_outlasts_the_lattice_exhale() {
        use vo::scene::lattice::{EXHALE_AFTER, EXHALE_OVER, LIVE_SLACK};
        let quiet_motion = (EXHALE_AFTER + EXHALE_OVER).max(LIVE_SLACK);
        assert!(LONG_SILENCE >= quiet_motion + 1.5, "{LONG_SILENCE} s of silence");
    }

    #[test]
    fn every_moment_records_then_ends_inside_its_duration() {
        let look = test_look();
        for &m in Moment::ALL {
            let s = scenario(&look, m);
            let f = files(&s);
            assert!(f.iter().any(|(_, f)| f.recording), "{m:?} never records");
            let (last_t, last) = f.last().unwrap();
            assert!(!last.recording && !last.transcribing, "{m:?} does not stop");
            assert!(*last_t < s.duration, "{m:?} ends after its duration");
            assert!(
                s.inputs.windows(2).all(|w| w[0].0 <= w[1].0),
                "{m:?} is not in time order"
            );
            // Overlapping stretches of sound would break the sequence, and
            // the popup would start its waveform again.
            let seqs: Vec<u32> = s
                .inputs
                .iter()
                .filter_map(|(_, i)| match i {
                    Input::Msg(Msg::Audio(f, _)) => Some(f.clone()),
                    _ => None,
                })
                .flatten()
                .filter(|f| !f.is_pump_silence())
                .map(|f| f.seq)
                .collect();
            assert!(
                seqs.windows(2).all(|w| w[1] == w[0] + 1),
                "{m:?} breaks the frame sequence"
            );
        }
    }

    #[test]
    fn moments_end_the_way_they_say() {
        let look = test_look();
        let outcome = |m| files(&scenario(&look, m)).last().unwrap().1.outcome;
        assert_eq!(outcome(Moment::Cancel), Some(Outcome::Cancelled));
        assert_eq!(outcome(Moment::NoSpeech), Some(Outcome::Empty));
        assert_eq!(outcome(Moment::Failure), Some(Outcome::Failed));
        assert_eq!(outcome(Moment::PastedEnter), Some(Outcome::Done));
        let last = |m| files(&scenario(&look, m)).last().unwrap().1.clone();
        let paste = last(Moment::PastedEnter).delivery.unwrap();
        assert!(paste.enter && paste.method == "paste");
        let typed = last(Moment::Typed).delivery.unwrap();
        assert!(!typed.enter && typed.chars == last(Moment::Typed).preview.chars().count());
        let no_speech = files(&scenario(&look, Moment::NoSpeech));
        assert!(no_speech.iter().all(|(_, f)| f.preview.is_empty()));
    }

    #[test]
    fn the_final_pass_changes_the_words_the_live_text_got_wrong() {
        let look = test_look();
        let f = files(&scenario(&look, Moment::FinalPass));
        let live = f.iter().rev().find(|(_, f)| f.recording).unwrap();
        let fin = &f.last().unwrap().1.preview;
        assert!(live.1.preview.contains("swish") && live.1.preview.contains("pop up"));
        assert!(fin.starts_with("Set the popup theme to swiss, so the waveform"));
        assert!(fin.ends_with('.'));
    }

    #[test]
    fn speech_has_onsets_and_pitch_and_the_word_times_follow_it() {
        let look = test_look();
        let s = scenario(&look, Moment::Pause);
        let voice: Vec<Voice> = s
            .inputs
            .iter()
            .filter_map(|(_, i)| match i {
                Input::Msg(Msg::Audio(_, v)) => Some(v.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(voice.iter().filter(|v| v.features.onset >= 0.5).count() > 20);
        assert!(voice
            .iter()
            .any(|v| v.features.voiced && v.features.f0_hz > 150.0));
        assert!(voice.windows(2).all(|w| w[1].clock > w[0].clock));
        assert!(voice.windows(2).all(|w| w[1].seq == w[0].seq + 1));
        let doc = files(&s).last().unwrap().1.live.clone().unwrap();
        let starts: Vec<f32> = doc.words.iter().filter_map(|w| w.start).collect();
        assert!(starts.windows(2).all(|w| w[1] >= w[0]));
        // The pause shows as a gap of over a second between two words.
        assert!(starts.windows(2).any(|w| w[1] - w[0] > 1.2));
    }

    #[test]
    fn the_lattice_moment_has_varied_pauses_a_comma_a_sentence_end_and_a_revision() {
        let look = test_look();
        let s = scenario(&look, Moment::Lattice);
        let f = files(&s);
        let doc = f.last().unwrap().1.live.clone().unwrap();
        let mut pauses: Vec<i32> = Vec::new();
        let mut end: Option<f32> = None;
        for w in &doc.words {
            if let (Some(a), Some(b)) = (w.start, w.end) {
                if let Some(e) = end {
                    pauses.push(((a - e) * 1000.0).round() as i32);
                }
                end = Some(b);
            }
        }
        pauses.sort();
        pauses.dedup();
        assert!(pauses.len() >= 4, "pauses: {pauses:?}");
        assert!(pauses.iter().any(|p| *p <= 150) && pauses.iter().any(|p| *p >= 800));
        let texts: Vec<&str> = f.iter().map(|(_, x)| x.preview.as_str()).collect();
        assert!(texts.iter().any(|t| t.ends_with("swish.")));
        assert!(f.last().unwrap().1.preview.contains("file, and"));
        assert!(f.last().unwrap().1.preview.contains("swiss. Then"));
    }

    /// M7.6 and R35: the edit moment pauses, moves the caret up two lines,
    /// types and presses Enter, and Swiss keeps the reading-line envelope
    /// with the caret line as its reading line.
    #[test]
    fn the_edit_moment_edits_within_the_envelope() {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut look = test_look();
        look.theme = OsdTheme::Swiss;
        let s = scenario(&look, Moment::Edit);
        let options = || Options {
            fonts: fonts.clone(),
            default_motion: default_motion(&look),
            ..Options::default()
        };
        let popup = || popup_for(Moment::Edit, Popup::scripted(look.clone(), &fonts));
        let keys: Vec<f64> = s
            .inputs
            .iter()
            .filter_map(|(t, i)| matches!(i, Input::Key(_)).then_some(*t))
            .collect();
        let (first, last) = (keys[0], *keys.last().unwrap());
        let mut feet = Vec::new();
        vo::envelope::sample_scenario(popup(), &s, 30.0, options(), &mut |_, t, dl| {
            feet.push((t, dl.edit_caret));
        });
        let at = |t: f64| feet.iter().rev().find(|f| f.0 <= t).unwrap().1;
        let before = at(first - 0.01).expect("the caret shows once the text is the user's");
        let up = at(first + 0.5).expect("the caret after two Ups");
        let typed = at(last - 0.1).expect("the caret after typing");
        let line = look.text_style().line_height;
        assert!(
            ((before.y - up.y) / line - 2.0).abs() < 0.1,
            "two lines up: {before:?} to {up:?}, lines {line}"
        );
        assert!(
            (typed.y - up.y).abs() < 1.0 && typed.x > up.x,
            "typed on that line: {typed:?}"
        );
        assert_eq!(at(s.duration), None, "Enter ends the edit");
        // At several frame rates, so the keys fall at every phase of a
        // frame and a pass does not hang on one frame timing; with one to
        // three lines up, so the typed word lands on different lines.
        for ups in 1..=3 {
            let s = edit_scenario(&look, ups);
            for fps in [30.0, 47.0, 60.0] {
                let report = vo::envelope::check_scenario(
                    popup(),
                    &s,
                    fps,
                    options(),
                    &vo::envelope::Envelope {
                        transcript: 0..1 << 20,
                        ..vo::envelope::Envelope::default()
                    },
                );
                assert!(report.is_ok(), "{ups} up at {fps} fps: {report}");
            }
        }
    }

    /// What [`EditSpy`] saw after each message, key and focus change.
    #[derive(Clone, Debug, Default)]
    struct EditSeen {
        t: f64,
        badge: Option<(&'static str, bool)>,
        drawn: bool,
        hint: Option<f32>,
        /// The marked words' text in the edit view, or in the shown words
        /// once the view is gone.
        marked: Vec<String>,
    }

    /// The popup, recording its edit badge, hint and marks.
    struct EditSpy(Popup, std::rc::Rc<std::cell::RefCell<Vec<EditSeen>>>);

    impl EditSpy {
        fn seen(&self, t: f64) {
            let p = &self.0;
            let marked = match &p.edit_view {
                Some(v) => v
                    .decor
                    .marks
                    .iter()
                    .map(|r| v.text[r.clone()].to_string())
                    .collect(),
                None => p
                    .shown
                    .iter()
                    .filter(|w| p.edit_marks.contains(&w.id))
                    .map(|w| w.text.clone())
                    .collect(),
            };
            self.1.borrow_mut().push(EditSeen {
                t,
                badge: p.edit_badge(),
                drawn: p.look_badge().is_some(),
                hint: p.edit_hint(),
                marked,
            });
        }
    }

    impl vo::App for EditSpy {
        type Msg = Msg;

        fn init(&mut self, cx: &mut Cx<Msg>) {
            self.0.init(cx);
        }

        fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
            self.0.update(msg, cx);
            self.seen(cx.t());
        }

        fn key(&mut self, ev: &vo::scene::input::KeyEvent, cx: &mut Cx<Msg>) -> bool {
            let used = self.0.key(ev, cx);
            self.seen(ev.t);
            used
        }

        fn keyboard_focus(&mut self, gained: bool, requested: bool, cx: &mut Cx<Msg>) {
            self.0.keyboard_focus(gained, requested, cx);
            self.seen(cx.t());
        }

        fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
            self.0.view(tokens)
        }

        fn surfaces(&self, tokens: &vo::Tokens) -> Vec<vo::SurfaceView<Msg>> {
            self.0.surfaces(tokens)
        }

        fn main_surface(&self) -> Option<(Size, vo::Placement)> {
            self.0.main_surface()
        }
    }

    fn spy_run(look: &Look, s: &vo::Scenario<Msg>) -> Vec<EditSeen> {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut p = popup_for(Moment::Edit, Popup::scripted(look.clone(), &fonts));
        p.live_surface = true;
        let spy = EditSpy(p, seen.clone());
        let options = Options {
            fonts: fonts.clone(),
            default_motion: default_motion(look),
            ..Options::default()
        };
        vo::envelope::sample_scenario(spy, s, 30.0, options, &mut |_, _, _| {});
        let out = seen.borrow().clone();
        out
    }

    fn edit_look() -> Look {
        let cfg = OsdConfig {
            keyboard_edit: true,
            ..OsdConfig::default()
        };
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        Look::new(&cfg, &plasma, String::new(), &fonts)
    }

    /// M7.7: the dimmed "EDIT" shows while a tap is pending and goes away
    /// when the tap is cancelled; with `keyboard_edit` off there is no
    /// badge strip at all.
    #[test]
    fn the_dimmed_edit_badge_shows_while_a_tap_is_pending() {
        let look = edit_look();
        let mut k = Take::new(0.2);
        k.record(0.2);
        let t = k.speak(0.2, &sentence_words(0, 6), 0.32);
        k.edit(t + 0.1, Some("pending"));
        // The daemon drops the block: a key came within 300 ms.
        k.edit(t + 0.4, None);
        let s = k.scenario(&look, t + 1.0);
        let seen = spy_run(&look, &s);
        let at = |t: f64| seen.iter().rev().find(|e| e.t <= t + 1e-9).unwrap();
        assert_eq!(
            (at(t).badge, at(t).drawn),
            (None, false),
            "no badge before the tap"
        );
        assert_eq!(
            at(t + 0.2).badge,
            Some(("EDIT", true)),
            "dimmed while pending"
        );
        assert!(at(t + 0.2).drawn);
        assert_eq!(
            (at(t + 0.6).badge, at(t + 0.6).drawn),
            (None, false),
            "gone on cancel"
        );
        assert!(look.live_surface().h > look.surface().h);
        let plain = test_look();
        assert_eq!(plain.live_surface().h, plain.surface().h, "off: no strip");
    }

    /// M7.7: the badge follows the edit, the hint shows while editing, and
    /// the typed word is marked, in the edit view and after Enter until the
    /// next recording.
    #[test]
    fn the_edit_badge_hint_and_marks_follow_the_edit() {
        let look = edit_look();
        let s = edit_scenario(&look, 2);
        let keys: Vec<f64> = s
            .inputs
            .iter()
            .filter_map(|(t, i)| matches!(i, Input::Key(_)).then_some(*t))
            .collect();
        let seen = spy_run(&look, &s);
        let at = |t: f64| seen.iter().rev().find(|e| e.t <= t + 1e-9).unwrap();
        let first = keys[0];
        let enter = *keys.last().unwrap() - 0.05;
        assert_eq!(at(first - 0.05).badge, Some(("EDIT \u{b7} paused", false)));
        let hint = at(first - 0.05).hint.expect("the hint on the first edit");
        assert!(hint > 0.6, "{hint}");
        assert_eq!(at(enter).marked, ["quickly"], "the typed word is marked");
        let released = seen
            .iter()
            .find(|e| e.t > enter + 0.05 && e.badge.is_some())
            .unwrap();
        assert_eq!(released.badge, Some(("sending", false)), "{released:?}");
        assert_eq!(seen.last().unwrap().badge, None);
        // The typed word stays from Enter through the release and the
        // delivery, underlined (prio-lane's M7.L7 finding).
        for e in seen.iter().filter(|e| e.t >= enter) {
            assert_eq!(e.marked, ["quickly"], "at {:.2} s", e.t);
        }
    }

    /// Plan 4.1: after the edit the marked words stay underlined, also when
    /// more speech arrives, until the next recording.
    #[test]
    fn edit_marks_stay_after_the_edit() {
        use vo::scene::scene::Scene;
        use vo::scene::text::ShapedLayout;
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut look = edit_look();
        look.theme = OsdTheme::Swiss;
        let mut p = Popup::scripted(look, &fonts);
        p.text("alpha beta gamma");
        let beta = p.shown[1].id;
        let shapes = |p: &Popup| {
            let mut scene: Scene<Msg> =
                Scene::new().with_text(Box::new(ShapedLayout::new(fonts.clone())));
            scene.set_viewport(0.0, p.look.live_surface(), 1.0);
            scene.apply(p.view(&vo::Tokens::default()), 1.0);
            scene.sample(2.0).shapes.len()
        };
        let plain = shapes(&p);
        p.edit_marks.insert(beta);
        p.text("alpha beta gamma delta");
        assert_eq!(
            p.shown[1].id, beta,
            "the word keeps its id as speech goes on"
        );
        assert!(shapes(&p) > plain, "the marked word is underlined");
    }
}
