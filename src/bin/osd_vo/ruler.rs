//! MO-41, remaining audio as a ruler: while the final text is made, a footer
//! ruler shows the seconds of audio left as units. Each finished unit closes
//! into a hairline; when the dictation is done the hairlines join into one
//! line ending in the final mark. It counts only what the daemon reports, so it
//! never runs backwards and never moves on its own.
//!
//! Opt-in per look with the `audio-ruler` motion feature. Swiss draws it in
//! place of its progress bar.

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::RRect;

use super::*;

/// The most units the ruler shows; longer audio puts several seconds in a unit.
const MAX_UNITS: usize = 24;
/// A finished unit closes into a hairline over this long.
const CLOSE_SECS: f64 = 0.15;
/// At the end the hairlines join over this long.
const JOIN_SECS: f64 = 0.22;
/// An open unit's height and a hairline's.
const UNIT_H: f32 = 5.0;
const HAIR_H: f32 = 1.0;

/// What the ruler remembers across the finishing phase.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ruler {
    /// Units across the ruler, set once at the first report.
    units: usize,
    /// Seconds of audio in one unit.
    per_unit: f32,
    /// The least audio left reported so far, so the ruler never runs backwards.
    left: f32,
    /// When each finished unit closed, in order.
    closed: Vec<f64>,
    /// When the dictation was done and the hairlines joined.
    joined: Option<f64>,
}

/// Seconds of audio left in a status line such as "Finishing: 4 s of audio left".
pub(crate) fn audio_left(status: &str) -> Option<f32> {
    let at = status.find(" s of audio left")?;
    let head = &status[..at];
    let num = head
        .rsplit(|c: char| !(c.is_ascii_digit() || c == '.'))
        .next()?;
    num.parse().ok()
}

impl Ruler {
    /// Takes a report of `left` seconds of audio left, with the fraction done when
    /// known, at scene time `t`.
    pub(crate) fn report(&mut self, left: f32, fraction: Option<f64>, t: f64) {
        if self.joined.is_some() || !left.is_finite() || left < 0.0 {
            return;
        }
        if self.units == 0 {
            // Audio already done before the first report counts as done units.
            let f = fraction.unwrap_or(0.0).clamp(0.0, 0.95) as f32;
            let total = (left / (1.0 - f)).max(left).max(1.0);
            self.units = (total.ceil() as usize).clamp(1, MAX_UNITS);
            self.per_unit = total / self.units as f32;
            self.left = total;
        }
        self.left = self.left.min(left);
        let open = (self.left / self.per_unit).ceil() as usize;
        let done = self.units.saturating_sub(open.min(self.units));
        while self.closed.len() < done {
            self.closed.push(t);
        }
    }

    /// The final text is there: every unit closes and the line joins.
    pub(crate) fn finish(&mut self, t: f64) {
        if self.units == 0 || self.joined.is_some() {
            return;
        }
        while self.closed.len() < self.units {
            self.closed.push(t);
        }
        self.joined = Some(t);
    }

    pub(crate) fn active(&self) -> bool {
        self.units > 0
    }

    /// Scene time after which the drawing stays still.
    fn until(&self) -> f64 {
        let last = self.closed.last().copied().unwrap_or(0.0) + CLOSE_SECS;
        self.joined.map_or(last, |j| last.max(j + JOIN_SECS))
    }

    /// Draws the ruler at `t` in `r`: `ink` for closed units and the line, `track`
    /// for open ones.
    fn paint(&self, t: f64, r: Rect, ink: Color, track: Color, reduced: bool, p: &mut Painter) {
        let n = self.units.max(1);
        let gap = 3.0f32.min(r.w / n as f32 * 0.3);
        let cw = (r.w - gap * (n - 1) as f32) / n as f32;
        let mid = r.y + r.h * 0.5;
        let k = |since: f64, secs: f64| -> f32 {
            if reduced {
                return if t >= since { 1.0 } else { 0.0 };
            }
            let x = ((t - since) / secs).clamp(0.0, 1.0) as f32;
            1.0 - (1.0 - x) * (1.0 - x)
        };
        let join = self.joined.map_or(0.0, |j| k(j, JOIN_SECS));
        for i in 0..n {
            let x = r.x + i as f32 * (cw + gap);
            let c = self.closed.get(i).map_or(0.0, |&at| k(at, CLOSE_SECS));
            let h = UNIT_H + (HAIR_H - UNIT_H) * c;
            // A closed unit reaches over its gap as the line joins.
            let w = cw + if i + 1 < n { gap * join } else { 0.0 };
            let colour = mix(track, ink, c);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x, mid - h * 0.5, w, h), 0.0),
                Paint::solid(colour),
            ));
        }
        if join > 0.0 {
            // The final mark: a short upright at the end of the line.
            let mh = 7.0 * join;
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(r.x + r.w - 1.5, mid - mh * 0.5, 1.5, mh), 0.0),
                Paint::solid(ink),
            ));
        }
    }
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

impl Popup {
    /// Feeds the ruler from the status line and the word facts, after the files changed.
    pub(crate) fn sync_ruler(&mut self, t: f64) {
        if !self.look.on("audio-ruler") {
            return;
        }
        if self.busy() && self.files.recording {
            self.life.ruler = Ruler::default();
            return;
        }
        let reported = self
            .files
            .live
            .as_ref()
            .and_then(|d| d.status.as_ref())
            .and_then(|s| s.audio_left_s)
            .or_else(|| audio_left(&self.files.status));
        if let Some(left) = reported {
            self.life.ruler.report(left, self.files.fraction, t);
        }
        if self.ending() == Some(Ending::Done) {
            self.life.ruler.finish(t);
        }
    }

    /// The ruler, once the daemon has reported audio left; `None` otherwise.
    pub(crate) fn audio_ruler(&self, ink: Color, track: Color) -> Option<Element<Msg>> {
        let r = &self.life.ruler;
        if !self.look.on("audio-ruler") || !r.active() {
            return None;
        }
        let r = r.clone();
        let reduced = kit::reduced() || self.look.on("reduced-motion");
        Some(
            Element::canvas("ruler", r.until(), move |t, rect, p| {
                r.paint(t, rect, ink, track, reduced, p)
            })
            .height(8.0)
            .grow(1.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_seconds_left_from_the_status() {
        assert_eq!(audio_left("Finishing: 4 s of audio left"), Some(4.0));
        assert_eq!(
            audio_left("Finishing: 12 s of audio left, about 1.2 s"),
            Some(12.0)
        );
        assert_eq!(audio_left("Finishing: loading the model"), None);
    }

    #[test]
    fn units_close_as_audio_is_done_and_never_reopen() {
        let mut r = Ruler::default();
        r.report(4.0, Some(0.0), 1.0);
        assert_eq!((r.units, r.closed.len()), (4, 0));
        r.report(2.5, None, 2.0);
        assert_eq!(r.closed, vec![2.0]);
        // A later report of more audio left does not run the ruler back.
        r.report(3.5, None, 3.0);
        assert_eq!(r.closed.len(), 1);
        r.report(0.0, None, 4.0);
        assert_eq!(r.closed.len(), 4);
        r.finish(5.0);
        assert_eq!(r.joined, Some(5.0));
    }

    #[test]
    fn audio_done_before_the_first_report_counts() {
        let mut r = Ruler::default();
        // Half done with 3 s left: 6 s in all, 3 units already closed.
        r.report(3.0, Some(0.5), 1.0);
        assert_eq!((r.units, r.closed.len()), (6, 3));
    }

    #[test]
    fn long_audio_shares_units() {
        let mut r = Ruler::default();
        r.report(120.0, None, 0.0);
        assert_eq!(r.units, MAX_UNITS);
        assert!((r.per_unit - 5.0).abs() < 1e-6);
    }
}
