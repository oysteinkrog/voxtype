//! The `swiss` OSD look, shared by the GTK4 popup and the vo GPU popup:
//! sizes, the waveform level curve, the accent hues, the status line and the
//! stop hint. Each frontend only draws.

use crate::config::{ActivationMode, HotkeyConfig};
use crate::osd::visual::{peak_meter_fraction, Color};

/// Height of the waveform strip.
pub const WAVE_HEIGHT_PX: i32 = 40;

/// Transcript size when `preview_font_pt` is unset.
pub const TEXT_PT: f32 = 18.0;

/// Most transcript lines. The text area grows a line at a time up to this,
/// and never shrinks during one dictation.
pub const MAX_LINES: i32 = 5;

/// Left plus right padding and border of the panel.
pub const PANEL_INSET_PX: i32 = 46;

/// dBFS floor of the waveform level curve.
const FLOOR_DBFS: f32 = -60.0;

/// Wave height, 0.0..=1.0, from a sample peak. Uses a -60..0 dBFS scale
/// raised to 1.5, so speech fills the strip and room noise stays near the line.
/// `waveform_gain` scales relative to its default of 10.
pub fn tick_level(amp: f64, gain: f64) -> f64 {
    let amp = amp * gain / 10.0;
    if amp <= 0.0 {
        return 0.0;
    }
    let dbfs = 20.0 * amp.log10();
    let level = peak_meter_fraction(dbfs as f32, FLOOR_DBFS) as f64;
    level.powf(1.5)
}

/// Level per frame, a 5-frame peak hold so speech keeps its full height,
/// then a 3-tap mean so the outline reads as one smooth shape.
pub fn smooth_levels(raw: &[f64]) -> Vec<f64> {
    if raw.is_empty() {
        return Vec::new();
    }
    let window = |len: usize, i: usize, r: usize| (i.saturating_sub(r), (i + r).min(len - 1));
    let peak: Vec<f64> = (0..raw.len())
        .map(|i| {
            let (lo, hi) = window(raw.len(), i, 2);
            raw[lo..=hi].iter().fold(0.0_f64, |a, b| a.max(*b))
        })
        .collect();
    (0..peak.len())
        .map(|i| {
            let (lo, hi) = window(peak.len(), i, 1);
            peak[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64
        })
        .collect()
}

/// Waveform colours left to right while recording: a cooler hue, the
/// lightened accent, a warmer hue.
pub fn wave_hues(accent: Color) -> (Color, Color, Color) {
    (
        shift_hue(accent, -40.0, 0.12),
        lighten(accent, 0.12),
        shift_hue(accent, 45.0, 0.12),
    )
}

/// Gradient stops across the waveform, as `(position, colour, alpha)`.
/// Older audio fades out toward the left edge.
pub fn wave_stops(left: Color, center: Color, right: Color) -> [(f32, Color, f32); 4] {
    [
        (0.0, left, 0.0),
        (0.2, left, 0.95),
        (0.6, center, 1.0),
        (1.0, right, 1.0),
    ]
}

/// Mix `c` toward white by `amount` (0..1).
pub fn lighten(c: Color, amount: f32) -> Color {
    Color::rgb(
        c.r + (1.0 - c.r) * amount,
        c.g + (1.0 - c.g) * amount,
        c.b + (1.0 - c.b) * amount,
    )
}

/// Rotate the hue of `c` by `degrees` and lighten it, keeping saturation.
pub fn shift_hue(c: Color, degrees: f32, amount: f32) -> Color {
    let (r, g, b) = (c.r, c.g, c.b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return lighten(c, amount);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut hue = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    hue = (hue + degrees).rem_euclid(360.0);
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let h = hue / 360.0;
    lighten(
        Color::rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)),
        amount,
    )
}

/// The status line from the daemon's status text, such as "Finishing: 4 s
/// of audio left", or empty while recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// "Listening", "Loading", "Finishing".
    pub state: String,
    /// "  ·  tap Right Ctrl to stop", "  ·  4 s of audio left", or empty.
    pub detail: String,
    /// The live model loads while still recording: the waveform keeps its
    /// colour and the clock keeps running.
    pub loading: bool,
    /// The final text is being made: the waveform turns grey and the dot
    /// hollow.
    pub finishing: bool,
}

impl StatusLine {
    /// `listening_detail` is shown while recording, for example the stop
    /// hint from [`stop_hint`] (empty for none).
    pub fn new(status: &str, listening_detail: &str) -> StatusLine {
        let (state, detail) = if status.is_empty() {
            let detail = if listening_detail.is_empty() {
                String::new()
            } else {
                format!("  ·  {listening_detail}")
            };
            ("Listening".to_string(), detail)
        } else {
            match status.split_once(": ") {
                Some((head, rest)) => (head.to_string(), format!("  ·  {rest}")),
                None => (status.to_string(), String::new()),
            }
        };
        let loading = status.starts_with("Loading");
        StatusLine {
            state,
            detail,
            loading,
            finishing: !status.is_empty() && !loading,
        }
    }

    /// The progress bar shows for any status.
    pub fn shows_progress(&self) -> bool {
        self.loading || self.finishing
    }
}

/// "tap Right Ctrl to stop" from `[hotkey]`, or empty when the hotkey is
/// off (compositor bindings call `voxtype record` instead).
pub fn stop_hint(hotkey: &HotkeyConfig) -> String {
    if !hotkey.enabled {
        return String::new();
    }
    let key = hotkey
        .modifiers
        .iter()
        .chain(std::iter::once(&hotkey.key))
        .map(|k| key_label(k))
        .collect::<Vec<_>>()
        .join("+");
    match hotkey.mode {
        ActivationMode::PushToTalk => format!("release {key} to stop"),
        ActivationMode::Toggle if hotkey.double_tap => format!("double tap {key} to stop"),
        ActivationMode::Toggle => format!("tap {key} to stop"),
    }
}

/// "RIGHTCTRL" -> "Right Ctrl", "KEY_SCROLLLOCK" -> "Scrolllock".
pub fn key_label(name: &str) -> String {
    let upper = name.trim().to_ascii_uppercase();
    let upper = upper.strip_prefix("KEY_").unwrap_or(&upper);
    let (side, rest) = match (upper.strip_prefix("RIGHT"), upper.strip_prefix("LEFT")) {
        (Some(r), _) if !r.is_empty() => ("Right ", r),
        (_, Some(r)) if !r.is_empty() => ("Left ", r),
        _ => ("", upper),
    };
    let rest = match rest {
        "CTRL" | "CONTROL" => "Ctrl".to_string(),
        "META" | "SUPER" => "Super".to_string(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_string() + &chars.as_str().to_ascii_lowercase())
                .unwrap_or_default()
        }
    };
    format!("{side}{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_labels_read_like_keycaps() {
        assert_eq!(key_label("RIGHTCTRL"), "Right Ctrl");
        assert_eq!(key_label("KEY_LEFTALT"), "Left Alt");
        assert_eq!(key_label("SCROLLLOCK"), "Scrolllock");
        assert_eq!(key_label("RIGHT"), "Right");
        assert_eq!(key_label("leftmeta"), "Left Super");
    }

    #[test]
    fn stop_hint_follows_mode() {
        let mut hk = HotkeyConfig {
            key: "RIGHTCTRL".into(),
            ..HotkeyConfig::default()
        };
        hk.mode = ActivationMode::PushToTalk;
        assert_eq!(stop_hint(&hk), "release Right Ctrl to stop");
        hk.mode = ActivationMode::Toggle;
        assert_eq!(stop_hint(&hk), "tap Right Ctrl to stop");
        hk.double_tap = true;
        assert_eq!(stop_hint(&hk), "double tap Right Ctrl to stop");
        hk.enabled = false;
        assert_eq!(stop_hint(&hk), "");
    }

    #[test]
    fn tick_level_maps_speech_high_and_silence_low() {
        assert_eq!(tick_level(0.0, 10.0), 0.0);
        assert!(tick_level(0.001, 10.0) < 0.05);
        assert!(tick_level(0.3, 10.0) > 0.6);
        assert!(tick_level(0.03, 20.0) > tick_level(0.03, 10.0));
    }

    #[test]
    fn smoothing_holds_peaks_then_averages() {
        let raw = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let s = smooth_levels(&raw);
        assert_eq!(s.len(), raw.len());
        assert_eq!(s[3], 1.0);
        assert!(s[0] > 0.0 && s[0] < 1.0);
        assert_eq!(s[8], 0.0);
    }

    #[test]
    fn status_line_states() {
        let l = StatusLine::new("", "tap Right Ctrl to stop");
        assert_eq!(l.state, "Listening");
        assert_eq!(l.detail, "  ·  tap Right Ctrl to stop");
        assert!(!l.shows_progress());
        let l = StatusLine::new("Loading: live text model, 2 s", "");
        assert!(l.loading && !l.finishing && l.shows_progress());
        assert_eq!(l.detail, "  ·  live text model, 2 s");
        let l = StatusLine::new("Finishing: 4 s of audio left", "");
        assert!(l.finishing && !l.loading);
        assert_eq!(l.state, "Finishing");
    }
}
