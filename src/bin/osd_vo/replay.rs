//! Recording real dictations and playing them back, and the popup in the vo preview window.
//!
//! `--record DIR` writes each dictation's messages (audio frames, voice features and the
//! runtime files) to `DIR/<unix ms>.jsonl` as the popup receives them, at the popup's own
//! time (`vo::record`). `--preview --replay FILE` plays one back in the preview window, and
//! `--preview` alone plays the scripted dictation of `--offline`. In the preview the timeline
//! scrubs, tokens and WGSL reload, a mock-up can go over the popup (`--reference`) and the
//! effects overlay shows what each effect costs.

use std::path::Path;

use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "osd-vo-preview"))]
use voxtype::audio::features::{AudioFeatures, MEL_BANDS};
#[cfg(any(test, feature = "osd-vo-preview"))]
use voxtype::audio::levels::AudioFrame;
use voxtype::osd::live::LiveDoc;

use super::Msg;
#[cfg(any(test, feature = "osd-vo-preview"))]
use super::{Files, Voice};

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
    /// The recorded form of `msg`, for the messages a recording keeps.
    pub(super) fn of(msg: &Msg) -> Option<Recorded> {
        Some(match msg {
            Msg::Audio(frames) => Recorded::Audio(
                frames
                    .iter()
                    .map(|f| (f.seq, f.min, f.max, f.peak_dbfs))
                    .collect(),
            ),
            Msg::Voice(v) => Recorded::Voice(
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
                    })
                    .collect(),
            ),
            Msg::Files(f) => Recorded::Files(Box::new(FilesRec {
                recording: f.recording,
                finishing: f.finishing,
                suppressed: f.suppressed,
                preview: f.preview.clone(),
                fraction: f.fraction,
                status: f.status.clone(),
                live: f.live.clone(),
            })),
            _ => return None,
        })
    }

    // `..Default::default()` covers fields that Files and AudioFeatures gain after a
    // recording was made.
    #[allow(clippy::needless_update)]
    #[cfg(any(test, feature = "osd-vo-preview"))]
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
            ),
            Recorded::Voice(v) => Msg::Voice(
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
    let (scenario, marks) = match value("--replay") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            let events: Vec<(f64, Recorded)> =
                vo::record::load(&path).map_err(anyhow::Error::msg)?;
            anyhow::ensure!(!events.is_empty(), "{} holds no messages", path.display());
            let marks = markers(&events);
            let msgs = events.into_iter().map(|(t, r)| (t, r.msg())).collect();
            (vo::record::scenario(look.surface(), msgs, 1.0), marks)
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
        move || super::Popup::new(look.clone(), &app_fonts),
        move || vo::Options {
            fonts: fonts.clone(),
            default_motion: motion,
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
    use super::*;

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
        }];
        let files = Files {
            recording: true,
            preview: "open the file".into(),
            fraction: Some(0.5),
            ..Files::default()
        };
        for msg in [Msg::Audio(frames), Msg::Voice(voice), Msg::Files(files)] {
            let rec = Recorded::of(&msg).expect("kept");
            let json = serde_json::to_string(&rec).unwrap();
            let back: Recorded = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", back.msg()), format!("{msg:?}"));
        }
        assert!(
            Recorded::of(&Msg::Check).is_none(),
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
            .unwrap()
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
