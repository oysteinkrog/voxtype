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
}

impl Moment {
    pub(crate) const ALL: &'static [Moment] = &[
        Moment::Dictation,
        Moment::FirstWord,
        Moment::Pause,
        Moment::LongSilence,
        Moment::RevisionBurst,
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
    ];

    /// The name on the command line.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Moment::Dictation => "dictation",
            Moment::FirstWord => "first-word",
            Moment::Pause => "pause",
            Moment::LongSilence => "long-silence",
            Moment::RevisionBurst => "revision-burst",
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
            unsure: vec!["popup", "swish", "confi"],
        }
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
        Moment::Dictation => return super::scenario(look, false),
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
            k.sound(t, t + 5.0, Sound::Quiet);
            let t = k.speak(t + 5.0, &sentence_words(3, 5), per);
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
                &["open", "the", "confirmation", "file", "and", "set"],
                per,
            );
            // A burst: the newest words rewritten four times in under a
            // second, one word becoming two and two becoming one, and a word
            // on the older text put right.
            let burst = [
                "open the confirmation file and set the pop",
                "open the confirmation file and set the pop up",
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
    let n = vo::render_offline_with(Popup::scripted(look, &fonts), &s, fps, dir, options)
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
        ..Options::default()
    };
    let spec = live_spec(cfg, &look);
    let played = Played {
        name: m.name(),
        popup: Popup::scripted(look, &fonts),
        inputs,
    };
    vo::run_overlay_with(played, spec, options).map_err(|e| anyhow::anyhow!("live moment: {e}"))
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
}
