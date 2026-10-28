//! The time-limit hairline (MO-35): in the last 10 s before the recording
//! limit, a hairline under the reading line shortens from the right at the
//! real rate. Its colour changes once, at 3 s left.
//!
//! Opt-in per look with the `limit-hairline` motion feature. The limit is
//! the voxtype config's `audio.max_duration_secs` (60 s by default); a
//! `limit-secs-N` feature overrides it, so a station tile can show the end
//! of a short scripted recording. Under reduced motion the line steps once
//! a second instead of gliding.

use super::kit::{ease_out, reduced};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};

/// The hairline shows for the last this many seconds.
pub(crate) const WINDOW_SECS: f64 = 10.0;
/// It changes colour with this many seconds left.
pub(crate) const WARN_SECS: f64 = 3.0;
/// It fades out this fast once the recording stops.
const FADE_SECS: f64 = 0.3;
/// How far under the newest line's baseline it runs, and how thick.
const DROP: f32 = 5.0;
const WIDTH: f32 = 1.0;

/// The recording limit from the voxtype config, read once.
fn config_limit() -> f64 {
    static LIMIT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var_os("VOXTYPE_CONFIG")
            .map(PathBuf::from)
            .or_else(VoxtypeConfig::default_path)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .and_then(|v| v.get("audio")?.get("max_duration_secs")?.as_integer())
            .map_or(60.0, |n| n as f64)
    })
}

/// The hairline's share of its full length with `left` seconds to go: 1
/// at the start of the window, 0 at the limit. Reduced motion steps it
/// once a second.
pub(crate) fn share(left: f64, still: bool) -> f32 {
    let left = if still { left.ceil() } else { left };
    (left / WINDOW_SECS).clamp(0.0, 1.0) as f32
}

/// The hairline, fixed for one view: where it runs and when.
#[derive(Clone)]
pub(crate) struct Hairline {
    /// Its left end and full length, in the canvas.
    pub from: Point,
    pub len: f32,
    /// When the recording started and when it hits the limit.
    pub start: f64,
    pub limit: f64,
    /// When the recording stopped, if it has.
    pub stopped: Option<f64>,
    pub color: Color,
    pub warn: Color,
    pub still: bool,
}

impl Hairline {
    pub(crate) fn until(&self) -> f64 {
        let end = self.start + self.limit;
        match self.stopped {
            Some(s) => s.min(end) + FADE_SECS,
            None => end + FADE_SECS,
        }
    }

    pub(crate) fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let now = self.stopped.map_or(t, |s| t.min(s));
        let left = self.start + self.limit - now;
        if left > WINDOW_SECS {
            return;
        }
        let fade = match self.stopped {
            Some(s) if !self.still => 1.0 - ease_out((t - s) / FADE_SECS),
            Some(_) => 0.0,
            None => 1.0,
        };
        let len = self.len * share(left, self.still);
        if fade <= 0.0 || len <= 0.0 {
            return;
        }
        let c = if left <= WARN_SECS { self.warn } else { self.color };
        p.shape(ShapeInstance::rrect(
            RRect::new(
                Rect::new(r.x + self.from.x, r.y + self.from.y, len, WIDTH),
                0.0,
            ),
            Paint::solid(c.fade(fade)),
        ));
    }
}

impl Popup {
    /// The recording limit in seconds, when this look shows the hairline.
    pub(crate) fn limit_secs(&self) -> Option<f64> {
        if !self.motion("limit-hairline") {
            return None;
        }
        let over = self
            .look
            .features
            .iter()
            .find_map(|f| f.strip_prefix("limit-secs-")?.parse::<f64>().ok());
        Some(over.unwrap_or_else(config_limit))
    }

    /// The time-limit hairline under the newest line of the transcript,
    /// whose top left is at `origin` in a canvas `size` large, or `None`
    /// when the look does not show it.
    pub(crate) fn limit_line(
        &self,
        origin: Point,
        size: Size,
        color: Color,
        warn: Color,
    ) -> Option<Element<Msg>> {
        let limit = self.limit_secs()?;
        let style = self.look.text_style();
        let width = self.tally_text_width();
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let shaped = self.measure.layout(&text, &style, Some(width));
        let baseline = shaped
            .lines
            .last()
            .map_or(style.line_height * 0.8, |l| l.baseline);
        let stopped = self
            .extra
            .finish_at
            .or(self.ended_at)
            .or_else(|| self.status().finishing.then_some(self.extra.now));
        let line = Hairline {
            from: Point::new(origin.x, origin.y + baseline + DROP),
            len: width,
            start: self.shown_at,
            limit,
            stopped,
            color,
            warn,
            still: self.still() || reduced(),
        };
        Some(
            kit::at(
                Element::canvas("limit", line.until(), move |t, r, p| line.draw(t, r, p)),
                0.0,
                0.0,
            )
            .size(size.w, size.h),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_shortens_over_the_last_ten_seconds() {
        assert_eq!(share(12.0, false), 1.0);
        assert_eq!(share(10.0, false), 1.0);
        assert!((share(5.0, false) - 0.5).abs() < 1e-6);
        assert_eq!(share(0.0, false), 0.0);
        // Reduced motion steps once a second.
        assert_eq!(share(4.2, true), share(5.0, false));
    }

    #[test]
    fn it_fades_after_the_stop_and_is_gone_by_its_until() {
        let line = Hairline {
            from: Point::new(0.0, 0.0),
            len: 100.0,
            start: 0.0,
            limit: 20.0,
            stopped: Some(15.0),
            color: Color::WHITE,
            warn: Color::WHITE,
            still: false,
        };
        assert!((line.until() - (15.0 + FADE_SECS)).abs() < 1e-9);
        let open = Hairline { stopped: None, ..line };
        assert!((open.until() - (20.0 + FADE_SECS)).abs() < 1e-9);
    }
}
