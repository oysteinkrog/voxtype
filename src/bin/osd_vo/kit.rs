//! Small pieces shared by the looks rebuilt from the mock-ups (`swissfi`,
//! `mission`, `dotmatrix`, `palette`): what the popup shows, easing, and the
//! level of the audio at a time.

use super::*;

use vo::scene::glass::GlassShape;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// What the popup shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Loading,
    Listening,
    Finishing,
    Done,
    Cancelled,
}

impl Popup {
    /// What the popup shows now. A dictation that ended with nothing typed
    /// shows as a cancel, even after it was finishing.
    pub(crate) fn kit_mode(&self) -> Mode {
        let status = self.status();
        if self.kit_nothing_typed() {
            Mode::Cancelled
        } else if self.extra.final_at.is_some() {
            Mode::Done
        } else if self.ended() {
            if self.extra.finish_at.is_some() {
                Mode::Done
            } else {
                Mode::Cancelled
            }
        } else if status.loading {
            Mode::Loading
        } else if status.finishing || self.extra.finish_at.is_some() {
            Mode::Finishing
        } else {
            Mode::Listening
        }
    }

    /// The dictation ended with nothing typed: a cancel, no speech or a
    /// failure, even after it was finishing.
    pub(crate) fn kit_nothing_typed(&self) -> bool {
        matches!(
            self.ending(),
            Some(Ending::Cancelled | Ending::Empty | Ending::Failed)
        )
    }

    /// When the current mode began, as far as the shared times tell.
    pub(crate) fn kit_mode_at(&self, mode: Mode) -> f64 {
        let x = &self.extra;
        match mode {
            Mode::Loading | Mode::Listening => self.shown_at,
            Mode::Finishing => x.finish_at.unwrap_or(self.shown_at),
            Mode::Done => x.final_at.or(x.finish_at).unwrap_or(self.shown_at),
            // The audio stops at the cancel.
            Mode::Cancelled => self.last_audio.unwrap_or(x.now),
        }
    }

    /// The fraction of the progress bar, while one shows.
    pub(crate) fn kit_progress(&self) -> Option<f32> {
        let status = self.status();
        self.files
            .fraction
            .map(|f| f.clamp(0.0, 1.0) as f32)
            .filter(|_| self.shows_progress(&status))
    }

    /// The app the text goes to, when the daemon says.
    pub(crate) fn kit_target(&self) -> Option<&str> {
        self.files
            .live
            .as_ref()
            .and_then(|d| d.target.as_ref())
            .map(|t| t.app.as_str())
            .filter(|a| !a.is_empty())
    }

    /// The raw levels, one per audio frame and not smoothed, with the
    /// same timing: for looks that show each syllable.
    pub(crate) fn kit_raw_levels(&self) -> Levels {
        Levels {
            levels: self.levels.iter().map(|&l| l as f32).collect(),
            newest_at: self.newest_at,
        }
    }

    /// The waveform's levels with their timing, for a canvas.
    pub(crate) fn kit_levels(&self) -> Levels {
        Levels {
            levels: self.wave.clone(),
            newest_at: self.newest_at,
        }
    }
}

/// What the mock-up looks of this lane keep between messages.
#[derive(Default)]
pub(crate) struct State {
    /// The preview text at the last sync.
    preview: String,
    /// Seconds from the end of the newest word's audio to its text on
    /// screen, smoothed; `None` before the first word with a time.
    pub latency: Option<f32>,
    /// When the preview text last changed.
    pub preview_at: f64,
}

impl Popup {
    /// Brings the shared state up to date after a message.
    pub(crate) fn kit_sync(&mut self, t: f64) {
        if self.fam.preview == self.files.preview {
            return;
        }
        self.fam.preview.clone_from(&self.files.preview);
        self.fam.preview_at = t;
        let Some(doc) = self
            .files
            .live
            .as_ref()
            .filter(|d| d.text == self.files.preview)
        else {
            return;
        };
        let Some(end) = doc.words.iter().rev().find_map(|w| w.end) else {
            return;
        };
        // The audio received so far against where the newest word ends.
        let heard = self.received as f32 / FRAME_HZ as f32;
        let now = (heard - end).clamp(0.0, 9.9);
        self.fam.latency = Some(match self.fam.latency {
            Some(old) => old + (now - old) * 0.3,
            None => now,
        });
    }
}

/// What the looks show from the voxtype config: the engine, its model, how
/// the text goes out and the microphone.
pub(crate) struct Facts {
    pub engine: String,
    pub model: String,
    pub output: String,
    pub mic: String,
}

/// The config facts, read once.
pub(crate) fn facts() -> &'static Facts {
    static FACTS: std::sync::OnceLock<Facts> = std::sync::OnceLock::new();
    FACTS.get_or_init(|| {
        let doc: toml::Value = std::env::var_os("VOXTYPE_CONFIG")
            .map(PathBuf::from)
            .or_else(VoxtypeConfig::default_path)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(Default::default()));
        let get = |path: &[&str]| {
            let mut v = &doc;
            for k in path {
                v = v.get(k)?;
            }
            Some(v.clone())
        };
        let text = |path: &[&str]| get(path).and_then(|v| v.as_str().map(str::to_string));
        let engine = text(&["engine"]).unwrap_or_else(|| "whisper".into());
        let model = text(&[engine.as_str(), "model"])
            .map(|m| {
                let m = m.trim_end_matches('/');
                m.rsplit('/').next().unwrap_or(m).to_string()
            })
            .unwrap_or_default();
        let mode = text(&["output", "mode"]).unwrap_or_else(|| "type".into());
        let submit = get(&["output", "auto_submit"]).and_then(|v| v.as_bool()) == Some(true);
        let mut output = match mode.as_str() {
            "paste" => "Paste",
            "clipboard" => "Clipboard",
            _ => "Type",
        }
        .to_string();
        if submit {
            output.push_str(" + Enter");
        }
        let mic = text(&["audio", "device"])
            .filter(|d| !d.is_empty() && d != "default")
            .unwrap_or_else(|| "Default mic".into());
        let mut engine = engine;
        if let Some(c) = engine.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        Facts {
            engine,
            model,
            output,
            mic,
        }
    })
}

/// The smoothed levels and the time the newest one is shown.
#[derive(Clone)]
pub(crate) struct Levels {
    pub levels: Arc<[f32]>,
    pub newest_at: f64,
}

impl Levels {
    /// The level at time `s`, 0 before the first frame.
    pub(crate) fn at(&self, s: f64) -> f32 {
        let n = self.levels.len();
        if n == 0 {
            return 0.0;
        }
        let back = ((self.newest_at - s) * FRAME_HZ as f64).max(0.0);
        let i = (n - 1) as f64 - back;
        if i < 0.0 {
            return 0.0;
        }
        let (a, k) = (i.floor() as usize, (i - i.floor()) as f32);
        let b = (a + 1).min(n - 1);
        self.levels[a] * (1.0 - k) + self.levels[b] * k
    }
}

/// An element placed at `(x, y)` in its parent, out of the flow.
pub(crate) fn at<M>(e: Element<M>, x: f32, y: f32) -> Element<M> {
    e.layout(|s| {
        s.position = taffy::Position::Absolute;
        s.inset = taffy::Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        };
    })
}

pub(crate) fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

/// Linear mix of two colours.
pub(crate) fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

pub(crate) fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

pub(crate) fn ease_in_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (if k < 0.5 {
        4.0 * k * k * k
    } else {
        1.0 - (-2.0 * k + 2.0).powi(3) / 2.0
    }) as f32
}

/// Reduced motion, as the framework reads it.
pub(crate) fn reduced() -> bool {
    static R: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

/// Live words for a status detail. A word keeps its id while the state
/// stays, so "4 s of audio left" turning to "3 s" changes the number alone.
pub(crate) fn detail_words(state: &str, detail: &str) -> Vec<Word> {
    let base = state
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        << 6;
    detail
        .split_whitespace()
        .enumerate()
        .map(|(i, w)| Word {
            id: base | i as u32 & 63,
            text: w.to_string(),
        })
        .collect()
}

/// Turns on compositor blur behind a look's panel (EG-07): where the compositor
/// can blur, the desktop shows blurred through the panel's translucent fill,
/// inside `shape` in the panel's box. [`GlassShape::Rounded`] follows the
/// panel's corner radius; a chamfered or notched look passes its own corners
/// (`GlassShape::chamfer`, `GlassShape::Corners`). The region follows the
/// outline to within one physical pixel, on a rotated or tilted layer too, and
/// holds the panel only once it is half opaque. Where the compositor has no
/// blur the panel draws its plain fill and the host logs "no compositor blur"
/// once. Call it on the panel element, not the root: the root fills the whole
/// surface.
// No look calls it yet: each look adopts it in its own look or R2 bead.
#[allow(dead_code)]
pub(crate) fn frosted<M>(panel: Element<M>, shape: GlassShape) -> Element<M> {
    panel.blur_behind(shape)
}

#[cfg(test)]
mod frosted_tests {
    use super::*;
    use vo::scene::display::IRect;
    use vo::scene::glass::blur_rects;
    use vo::scene::scene::Scene;

    /// The swiss popup's panel, frosted with `shape`, sampled at 2 s: its blur
    /// region and the panel's box from the region's bounds.
    fn swiss_region(shape: GlassShape) -> (Vec<IRect>, [f32; 4]) {
        let cfg = OsdConfig {
            theme: OsdTheme::Swiss,
            width_px: 600,
            ..OsdConfig::default()
        };
        let theme = PlasmaTheme::default();
        let fonts = load_fonts(&theme).unwrap();
        let look = Look::new(&cfg, &theme, String::new(), &fonts);
        let popup = Popup::new(look, &fonts);
        let mut scene: Scene<Msg> = Scene::new();
        scene.set_viewport(0.0, Size::new(700.0, 400.0), 1.0);
        scene.apply(
            Element::column("root").child(frosted(popup.panel(), shape)),
            0.0,
        );
        let rects = scene.sample(2.0).blur_region().rects;
        assert!(!rects.is_empty(), "the frosted panel asks for blur");
        let x0 = rects.iter().map(|r| r.x).min().unwrap();
        let y0 = rects.iter().map(|r| r.y).min().unwrap();
        let x1 = rects.iter().map(|r| r.x + r.w).max().unwrap();
        let y1 = rects.iter().map(|r| r.y + r.h).max().unwrap();
        (
            rects,
            [x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32],
        )
    }

    #[test]
    fn a_frosted_swiss_panel_blurs_its_rounded_rect() {
        let (rects, rect) = swiss_region(GlassShape::Rounded);
        assert_eq!(rect[..3], [0.0, 0.0, 600.0], "the panel's box: {rect:?}");
        let mut want = Vec::new();
        blur_rects(rect, RADIUS, &mut want);
        assert_eq!(rects, want);
    }

    #[test]
    fn a_frosted_swiss_panel_follows_a_chamfered_outline() {
        let (rects, rect) = swiss_region(GlassShape::chamfer(12.0));
        let (w, h) = (rect[2] as i32, rect[3] as i32);
        assert!(rects.len() > 3, "{} rects", rects.len());
        for r in &rects {
            // Inside the octagon: a rect whose outer edge is `dy` from the
            // panel's top or bottom is inset by the chamfer left there, less
            // the one pixel the region may round out.
            let dy = r.y.min(h - (r.y + r.h)).max(0);
            let inset = (11 - dy).max(0);
            assert!(
                r.x >= inset && r.x + r.w <= w - inset,
                "{r:?} leaves the octagon"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::swiss_look::edit_check::fonts as test_fonts;
    use super::*;

    /// A dictation that was finishing and then failed, found no speech or
    /// was cancelled shows as a cancel, not as done; the looks that keep
    /// their own mode take it from here.
    #[test]
    fn an_ending_with_nothing_typed_is_not_done_after_finishing() {
        let reg = test_fonts();
        let cfg = OsdConfig {
            theme: OsdTheme::Dotmatrix,
            ..OsdConfig::default()
        };
        let look = Look::new(&cfg, &PlasmaTheme::default(), String::new(), &reg);
        for (outcome, mode) in [
            (Outcome::Failed, Mode::Cancelled),
            (Outcome::Empty, Mode::Cancelled),
            (Outcome::Cancelled, Mode::Cancelled),
            (Outcome::Done, Mode::Done),
        ] {
            let mut popup = Popup::new(look.clone(), &reg);
            popup.was_recording = true;
            popup.extra.finish_at = Some(1.0);
            popup.files.transcribing = true;
            assert_eq!(popup.kit_mode(), Mode::Finishing);
            popup.files.transcribing = false;
            popup.files.outcome = Some(outcome);
            assert_eq!(popup.kit_mode(), mode, "{outcome:?}");
            popup.extra.final_at = Some(2.0);
            assert_eq!(popup.kit_mode(), mode, "{outcome:?} with the final text");
        }
    }
}
