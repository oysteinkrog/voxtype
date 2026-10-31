//! Recording real dictations and playing them back, and the popup in the vo preview window.
//!
//! `--record DIR` writes each dictation's messages (audio frames, voice features and the
//! runtime files) to `DIR/<unix ms>.jsonl` as the popup receives them, at the popup's own
//! time (`vo::record`). `--preview --replay FILE` plays one back in the preview window, and
//! `--preview` alone plays the scripted dictation of `--offline`. In the preview the timeline
//! scrubs, tokens and WGSL reload, a mock-up can go over the popup (`--reference`) and the
//! effects overlay shows what each effect costs.
//!
//! `--offline DIR --replay FILE` renders a recording to frames, and `--envelope --replay FILE`
//! checks it. A recording is a finished dictation, so before it plays its words and when each
//! came are known ([`Lookahead`], EG-35): looks that show the sentence ahead use them. Live and
//! scripted runs have no lookahead.

use std::path::Path;

use serde::{Deserialize, Serialize};
use voxtype::audio::features::{AudioFeatures, MEL_BANDS};
use voxtype::audio::levels::AudioFrame;
use voxtype::osd::live::LiveDoc;

use super::{Files, Look, Msg, Voice};

/// One recorded message. Audio, voice features and the runtime files; timers are the
/// popup's own and are not recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Recorded {
    /// Frames as `[seq, min, max, peak dBFS]`.
    Audio(Vec<(u32, f32, f32, f32)>),
    Voice(Vec<VoiceRec>),
    Files(Box<FilesRec>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct VoiceRec {
    seq: u32,
    rms_dbfs: f32,
    f0_hz: f32,
    voiced: bool,
    centroid_hz: f32,
    flatness: f32,
    mel: Vec<u8>,
    #[serde(default)]
    onset: f32,
    #[serde(default)]
    peak_dbfs: f32,
    /// Samples since the stream began, 0 in recordings made before it was kept.
    #[serde(default)]
    clock: u64,
}

/// Fields added later read as their defaults, so old recordings still play.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FilesRec {
    recording: bool,
    finishing: bool,
    suppressed: bool,
    preview: String,
    fraction: Option<f64>,
    status: String,
    live: Option<LiveDoc>,
}

impl Recorded {
    /// The recorded form of `msg`, for the messages a recording keeps. A capture batch
    /// is kept as two lines, its frames and then their voice features, as recordings
    /// made before the two were one message hold them.
    pub(super) fn of(msg: &Msg) -> Vec<Recorded> {
        match msg {
            Msg::Audio(frames, voice) => {
                let mut recs = Vec::with_capacity(2);
                if !frames.is_empty() {
                    recs.push(Recorded::Audio(
                        frames
                            .iter()
                            .map(|f| (f.seq, f.min, f.max, f.peak_dbfs))
                            .collect(),
                    ));
                }
                if !voice.is_empty() {
                    recs.push(Recorded::voice(voice));
                }
                recs
            }
            Msg::Files(f) => vec![Recorded::Files(Box::new(FilesRec {
                recording: f.recording,
                finishing: f.finishing,
                suppressed: f.suppressed,
                preview: f.preview.clone(),
                fraction: f.fraction,
                status: f.status.clone(),
                live: f.live.clone(),
            }))],
            _ => Vec::new(),
        }
    }

    fn voice(v: &[Voice]) -> Recorded {
        Recorded::Voice(
            v.iter()
                .map(|v| VoiceRec {
                    seq: v.seq,
                    rms_dbfs: v.features.rms_dbfs,
                    f0_hz: v.features.f0_hz,
                    voiced: v.features.voiced,
                    centroid_hz: v.features.centroid_hz,
                    flatness: v.features.flatness,
                    mel: v.features.mel.to_vec(),
                    onset: v.features.onset,
                    peak_dbfs: v.peak_dbfs,
                    clock: v.clock,
                })
                .collect(),
        )
    }

    // `..Default::default()` covers fields that Files and AudioFeatures gain after a
    // recording was made.
    #[allow(clippy::needless_update)]
    pub(super) fn msg(self) -> Msg {
        match self {
            Recorded::Audio(frames) => Msg::Audio(
                frames
                    .into_iter()
                    .map(|(seq, min, max, peak_dbfs)| AudioFrame {
                        seq,
                        min,
                        max,
                        peak_dbfs,
                    })
                    .collect(),
                Vec::new(),
            ),
            Recorded::Voice(v) => Msg::Audio(
                Vec::new(),
                v.into_iter()
                    .map(|v| {
                        let mut mel = [0u8; MEL_BANDS];
                        let n = v.mel.len().min(MEL_BANDS);
                        mel[..n].copy_from_slice(&v.mel[..n]);
                        Voice {
                            seq: v.seq,
                            features: AudioFeatures {
                                rms_dbfs: v.rms_dbfs,
                                f0_hz: v.f0_hz,
                                voiced: v.voiced,
                                centroid_hz: v.centroid_hz,
                                flatness: v.flatness,
                                mel,
                                onset: v.onset,
                                ..AudioFeatures::default()
                            },
                            peak_dbfs: v.peak_dbfs,
                            clock: v.clock,
                        }
                    })
                    .collect(),
            ),
            Recorded::Files(f) => Msg::Files(Files {
                recording: f.recording,
                finishing: f.finishing,
                suppressed: f.suppressed,
                preview: f.preview,
                fraction: f.fraction,
                status: f.status,
                live: f.live,
                ..Files::default()
            }),
        }
    }
}

/// A finished dictation's live words and when each first showed, in the recording's time
/// (EG-35). Live, a word is known only once it is spoken; a replay knows them all.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Lookahead {
    pub words: Vec<(String, f64)>,
}

impl Lookahead {
    /// The words the recording last showed while recording, each timed by the first message
    /// that showed that many words. None when it shows none.
    pub(crate) fn scan(events: &[(f64, Recorded)]) -> Option<Lookahead> {
        let text = |f: &FilesRec| match &f.live {
            Some(doc) => doc.text.clone(),
            None => f.preview.clone(),
        };
        let last = events.iter().rev().find_map(|(_, r)| match r {
            Recorded::Files(f) if f.recording && !text(f).trim().is_empty() => Some(text(f)),
            _ => None,
        })?;
        let words: Vec<&str> = last.split_whitespace().collect();
        let mut times: Vec<Option<f64>> = vec![None; words.len()];
        for (t, r) in events {
            let Recorded::Files(f) = r else { continue };
            let n = text(f).split_whitespace().count().min(words.len());
            for slot in &mut times[..n] {
                slot.get_or_insert(*t);
            }
        }
        let end = events.last().map_or(0.0, |e| e.0);
        Some(Lookahead {
            words: words
                .iter()
                .zip(times)
                .map(|(w, t)| (w.to_string(), t.unwrap_or(end)))
                .collect(),
        })
    }

    /// The words after the first `shown` that have not come by `t`, in order.
    pub(crate) fn ahead(&self, shown: usize, t: f64) -> &[(String, f64)] {
        let rest = self.words.get(shown..).unwrap_or(&[]);
        &rest[rest.partition_point(|w| w.1 <= t)..]
    }
}

/// The messages of a recording made with `--record`.
pub(super) fn load(path: &Path) -> anyhow::Result<Vec<(f64, Recorded)>> {
    let events: Vec<(f64, Recorded)> = vo::record::load(path).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(!events.is_empty(), "{} holds no messages", path.display());
    Ok(events)
}

/// A recording played at its own pace, one second past its last message.
pub(super) fn scenario(look: &Look, events: Vec<(f64, Recorded)>) -> vo::Scenario<Msg> {
    let msgs = events.into_iter().map(|(t, r)| (t, r.msg())).collect();
    vo::record::scenario(look.surface(), msgs, 1.0)
}

/// `--offline DIR --replay FILE`: a recording rendered to PNG frames, with its words ahead.
pub(super) fn offline(
    look: Look,
    fonts: vo::FontRegistry,
    dir: &Path,
    fps: f64,
    file: &Path,
    gpu_times: bool,
) -> anyhow::Result<()> {
    let events = load(file)?;
    let lookahead = Lookahead::scan(&events);
    let scenario = scenario(&look, events);
    let (times, sink) = super::gpu_time_sink(gpu_times);
    let options = vo::Options {
        fonts: fonts.clone(),
        default_motion: super::default_motion(&look),
        frame_timings: sink,
        renderer_setup: Some(std::sync::Arc::new(vo_3d::install)),
        ..vo::Options::default()
    };
    let popup = super::Popup::replayed(look, &fonts, lookahead);
    let n = vo::render_offline_with(popup, &scenario, fps, dir, options)
        .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
    eprintln!("voxtype-osd-vo: wrote {n} frames to {}", dir.display());
    if gpu_times {
        super::print_gpu_times(&times.lock().unwrap());
    }
    Ok(())
}

/// A new recording for one dictation in `dir`, named by the time it starts.
pub(super) fn recorder(dir: &Path) -> Option<vo::record::Recorder> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{ms}.jsonl"));
    match vo::record::Recorder::create(&path) {
        Ok(r) => {
            tracing::info!("recording this dictation to {}", path.display());
            Some(r)
        }
        Err(e) => {
            tracing::warn!("cannot record to {}: {e}", path.display());
            None
        }
    }
}

/// Named times in a recording: where it starts recording, stops and goes idle.
#[cfg(any(test, feature = "osd-vo-preview"))]
fn markers(events: &[(f64, Recorded)]) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let (mut recording, mut finishing) = (false, false);
    for (t, r) in events {
        let Recorded::Files(f) = r else { continue };
        if f.recording && !recording {
            out.push(("rec".to_string(), *t));
        }
        if !f.recording && recording {
            out.push(("stop".to_string(), *t));
        }
        if !f.finishing && finishing && !f.recording {
            out.push(("done".to_string(), *t));
        }
        (recording, finishing) = (f.recording, f.finishing);
    }
    out
}

/// `voxtype-osd-vo --preview [--theme T] [--replay FILE | --cancel] [preview options]`, and
/// the other `vo::preview` modes (`--sheet`, `--compare`). Exits the process.
#[cfg(feature = "osd-vo-preview")]
pub(super) fn preview_main() -> anyhow::Result<()> {
    use voxtype::osd::config::OsdTheme;
    use voxtype::osd::plasma;

    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
    let config = value("--config")
        .or_else(|| std::env::var("VOXTYPE_CONFIG").ok())
        .map(std::path::PathBuf::from);
    let (mut cfg, hotkey) = super::load_config(config.as_deref());
    let theme_name = value("--theme").or_else(|| std::env::var("VOXTYPE_OSD_THEME").ok());
    if let Some(t) = theme_name.as_deref().and_then(OsdTheme::parse_str) {
        cfg.theme = t;
    }
    let theme = plasma::load();
    let fonts = super::load_fonts(&theme)?;
    let look = super::Look::new(&cfg, &theme, super::stop_hint(&hotkey), &fonts);
    let mut lookahead = None;
    let (scenario, marks) = match value("--replay") {
        Some(path) => {
            let events = load(std::path::Path::new(&path))?;
            let marks = markers(&events);
            lookahead = Lookahead::scan(&events);
            (scenario(&look, events), marks)
        }
        None => {
            let cancel = args.iter().any(|a| a == "--cancel");
            let marks = if cancel {
                vec![("words".into(), 1.2), ("cancel".into(), 3.4)]
            } else {
                vec![
                    ("words".into(), 1.2),
                    ("pause".into(), 3.4),
                    ("stop".into(), 6.6),
                    ("final".into(), 8.2),
                ]
            };
            (super::scenario(&look, cancel), marks)
        }
    };
    let motion = super::default_motion(&look);
    let name = format!(
        "voxtype-osd-vo-{}",
        format!("{:?}", cfg.theme).to_lowercase()
    );
    let app_fonts = fonts.clone();
    let mut p = vo::preview::Preview::new(
        name,
        move || {
            let mut p = super::Popup::new(look.clone(), &app_fonts);
            p.lookahead = lookahead.clone().map(std::sync::Arc::new);
            p
        },
        move || vo::Options {
            fonts: fonts.clone(),
            default_motion: motion,
            renderer_setup: Some(std::sync::Arc::new(vo_3d::install)),
            ..vo::Options::default()
        },
        scenario,
    )
    .fps(60.0);
    for (name, t) in marks {
        p = p.marker(name, t);
    }
    std::process::exit(vo::preview::main(p));
}

#[cfg(test)]
mod tests {
    use super::super::swiss_look::edit_check::fonts as test_fonts;
    use super::*;
    use vo::scene::display::{ItemKind, NO_WORD};
    use vo::Input;
    use voxtype::osd::config::{OsdConfig, OsdTheme};
    use voxtype::osd::plasma::PlasmaTheme;

    fn look(theme: OsdTheme) -> Look {
        let cfg = OsdConfig {
            theme,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &PlasmaTheme::default(), String::new(), &test_fonts());
        look.set_motion(&cfg.motion);
        look
    }

    /// The scripted dictation as `--record` would keep it: no real voice.
    fn scripted_recording(look: &Look) -> Vec<(f64, Recorded)> {
        super::super::scenario(look, false)
            .inputs
            .iter()
            .filter_map(|(t, i)| match i {
                Input::Msg(m) => Some((*t, m)),
                _ => None,
            })
            .flat_map(|(t, m)| Recorded::of(m).into_iter().map(move |r| (t, r)))
            .collect()
    }

    /// Writes the replay fixture, tests/fixtures/osd-vo/replay-dictation.jsonl, from the
    /// scripted dictation. Run it by hand after the script changes:
    /// `cargo test --features osd-vo --bin voxtype-osd-vo write_the_replay_fixture -- --ignored`.
    #[test]
    #[ignore]
    fn write_the_replay_fixture() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/osd-vo/replay-dictation.jsonl");
        let _ = std::fs::remove_file(&path);
        let rec = vo::record::Recorder::create(&path).unwrap();
        for (t, r) in scripted_recording(&look(OsdTheme::Karaoke)) {
            rec.record_at(t, &r).unwrap();
        }
    }

    #[test]
    fn the_fixture_plays_back_with_its_words_ahead() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/osd-vo/replay-dictation.jsonl");
        let events = load(&path).unwrap();
        let ahead = Lookahead::scan(&events).unwrap();
        assert_eq!(ahead.words.first().map(|w| w.0.as_str()), Some("open"));
        assert!(ahead.words.len() > 20);
    }

    #[test]
    fn the_lookahead_times_each_word_by_its_first_showing() {
        let files = |recording: bool, preview: &str| {
            Recorded::Files(Box::new(FilesRec {
                recording,
                preview: preview.into(),
                ..FilesRec::default()
            }))
        };
        let events = vec![
            (0.5, files(true, "")),
            (1.0, files(true, "open")),
            (1.4, files(true, "open the confi")),
            (1.8, files(true, "open the configuration")),
            (2.5, files(true, "open the configuration file")),
            (3.0, files(false, "Open the configuration file.")),
        ];
        let la = Lookahead::scan(&events).unwrap();
        let words: Vec<(&str, f64)> = la.words.iter().map(|(w, t)| (w.as_str(), *t)).collect();
        assert_eq!(
            words,
            [
                ("open", 1.0),
                ("the", 1.4),
                ("configuration", 1.4),
                ("file", 2.5)
            ]
        );
        // Ahead of two shown words at 1.5 s: only the word not come yet.
        assert_eq!(la.ahead(2, 1.5).len(), 1);
        assert_eq!(la.ahead(1, 1.0).len(), 3);
        assert!(la.ahead(4, 9.0).is_empty());
        assert!(la.ahead(9, 0.0).is_empty());
        assert!(Lookahead::scan(&[(0.1, files(true, ""))]).is_none());
    }

    /// The coming words karaoke draws at `t`, their ink, with or without the lookahead.
    fn karaoke_ahead_glyphs(replayed: bool) -> Vec<(f64, Vec<f32>)> {
        let look = look(OsdTheme::Karaoke);
        let events = scripted_recording(&look);
        let lookahead = replayed.then(|| Lookahead::scan(&events)).flatten();
        let scenario = scenario(&look, events);
        let fonts = test_fonts();
        let options = vo::Options {
            fonts: fonts.clone(),
            default_motion: super::super::default_motion(&look),
            ..vo::Options::default()
        };
        let popup = super::super::Popup::replayed(look, &fonts, lookahead);
        let mut out = Vec::new();
        vo::envelope::sample_scenario(popup, &scenario, 30.0, options, &mut |_, t, dl| {
            let mut inks = Vec::new();
            for l in &dl.layers {
                for item in &dl.items[l.items.start as usize..l.items.end as usize] {
                    if item.kind != ItemKind::Glyphs {
                        continue;
                    }
                    for g in &dl.glyphs[item.start as usize..(item.start + item.len) as usize] {
                        // White coming words; the card's labels are other colours.
                        let white = (g.color.r - g.color.b).abs() < 0.05 && g.color.r > 0.9;
                        if g.word == NO_WORD && white && g.color.a < 0.75 {
                            inks.push(g.color.a * l.opacity);
                        }
                    }
                }
            }
            out.push((t, inks));
        });
        out
    }

    /// EG-35: a replay shows the newest line's coming words at 35 % ink, none ever live or
    /// scripted, and each lights up in the 0.3 s before its time.
    #[test]
    fn karaoke_shows_the_coming_words_only_in_a_replay() {
        let scripted = karaoke_ahead_glyphs(false);
        let replayed = karaoke_ahead_glyphs(true);
        assert!(
            scripted.iter().all(|(_, g)| g.is_empty()),
            "no word before its time"
        );
        let look = look(OsdTheme::Karaoke);
        let la = Lookahead::scan(&scripted_recording(&look)).unwrap();
        // Letters of the first k coming words.
        let prefixes = |t: f64| -> Vec<usize> {
            let ahead: Vec<_> = la.words.iter().filter(|w| w.1 > t).collect();
            (1..=ahead.len())
                .map(|k| ahead[..k].iter().map(|w| w.0.chars().count()).sum())
                .collect()
        };
        let mut seen = 0;
        for (t, inks) in &replayed {
            if inks.is_empty() {
                continue;
            }
            let next = la
                .words
                .iter()
                .find(|w| w.1 > *t)
                .map_or(f64::INFINITY, |w| w.1);
            if next - t > AHEAD_SWEEP + 0.05 {
                assert!(
                    inks.iter().all(|a| (a - 0.35).abs() <= 0.02),
                    "t {t}: {inks:?}"
                );
                assert!(
                    prefixes(*t).contains(&inks.len()),
                    "t {t}: {} glyphs",
                    inks.len()
                );
                seen += 1;
            } else if next - t < AHEAD_SWEEP - 0.05 {
                assert!(inks.iter().any(|a| *a > 0.4), "t {t}: lights up");
            }
        }
        assert!(seen > 5, "{seen} frames with coming words");
    }

    const AHEAD_SWEEP: f64 = 0.3;

    #[test]
    fn recorded_messages_come_back_the_same() {
        let frames = vec![AudioFrame {
            seq: 7,
            min: -0.25,
            max: 0.5,
            peak_dbfs: -6.0,
        }];
        let mut mel = [0u8; MEL_BANDS];
        mel[3] = 200;
        let voice = vec![Voice {
            seq: 7,
            features: AudioFeatures {
                rms_dbfs: -20.0,
                f0_hz: 180.0,
                voiced: true,
                centroid_hz: 900.0,
                flatness: 0.2,
                mel,
                onset: 0.7,
            },
            peak_dbfs: -6.0,
            clock: 24_000,
        }];
        let files = Files {
            recording: true,
            preview: "open the file".into(),
            fraction: Some(0.5),
            ..Files::default()
        };
        // A capture batch is kept as its frames, then their voice features.
        let batch = Recorded::of(&Msg::Audio(frames.clone(), voice.clone()));
        assert_eq!(batch.len(), 2);
        let parts = [
            Msg::Audio(frames, Vec::new()),
            Msg::Audio(Vec::new(), voice),
        ];
        for (rec, msg) in batch.into_iter().zip(parts) {
            let json = serde_json::to_string(&rec).unwrap();
            let back: Recorded = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", back.msg()), format!("{msg:?}"));
        }
        let msg = Msg::Files(files);
        let [rec] = <[Recorded; 1]>::try_from(Recorded::of(&msg)).expect("kept");
        let json = serde_json::to_string(&rec).unwrap();
        let back: Recorded = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{:?}", back.msg()), format!("{msg:?}"));
        assert!(
            Recorded::of(&Msg::Check).is_empty(),
            "timers are not recorded"
        );
    }

    #[test]
    fn markers_follow_the_recording_state() {
        let files = |recording, finishing| {
            Recorded::of(&Msg::Files(Files {
                recording,
                finishing,
                ..Files::default()
            }))
            .remove(0)
        };
        let events = vec![
            (0.1, files(true, false)),
            (0.5, files(true, false)),
            (3.0, files(false, true)),
            (4.0, files(false, false)),
        ];
        let m = markers(&events);
        assert_eq!(
            m,
            vec![
                ("rec".to_string(), 0.1),
                ("stop".to_string(), 3.0),
                ("done".to_string(), 4.0)
            ]
        );
    }
}
