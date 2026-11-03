//! The final pass's progress bar between the worker's status lines.
//!
//! The live worker writes a status line when the recording stops and again
//! when each part of the clip is done. With one part left, as on most
//! dictations, that is one line, with the share done so far and an estimate
//! of the seconds left, and then nothing until the text comes: the bar
//! stood still for the whole final pass. This moves it on from the
//! estimate: 90% of the way at the estimated time, then ever slower towards
//! the end, so it never stops and never claims to be done.

/// The last share and estimate the worker sent, and when the popup saw them.
#[derive(Debug, Default)]
pub(crate) struct FinishProgress {
    /// The share done as the daemon wrote it, 0 to 1.
    pub raw: Option<f64>,
    eta: Option<f32>,
    at: f64,
    /// The wake-up for the next step of the bar, while one is armed.
    pub timer: Option<vo::TimerId>,
}

/// How often the view is rebuilt while the bar moves.
pub(crate) const STEP_HZ: f64 = 30.0;

impl FinishProgress {
    /// Notes the share and estimate in the runtime files at `t`.
    pub fn note(&mut self, raw: Option<f64>, eta: Option<f32>, t: f64) {
        if (raw, eta) != (self.raw, self.eta) {
            self.raw = raw;
            self.eta = eta;
            self.at = t;
        }
    }

    /// Whether the bar moves on its own: a share and an estimate are known.
    pub fn moving(&self) -> bool {
        self.raw.is_some() && self.eta.is_some_and(|e| e > 0.0)
    }

    /// The share to show at `t`.
    pub fn fraction(&self, t: f64) -> Option<f64> {
        let raw = self.raw?.clamp(0.0, 1.0);
        let Some(eta) = self.eta.filter(|e| *e > 0.0) else {
            return Some(raw);
        };
        let x = ((t - self.at) / eta as f64).max(0.0);
        Some(raw + (1.0 - raw) * ease(x))
    }
}

/// Of the share left, how much shows at `x` estimated durations: 90% at 1,
/// then the last 10% in an ever slower approach, never all of it.
fn ease(x: f64) -> f64 {
    if x <= 1.0 {
        0.9 * x
    } else {
        0.9 + 0.09 * (1.0 - (-(x - 1.0) * 2.0).exp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_moves_from_the_estimate_and_never_finishes() {
        let mut p = FinishProgress::default();
        assert_eq!(p.fraction(0.0), None);
        // 40% done, 2 s left, seen at t = 10.
        p.note(Some(0.4), Some(2.0), 10.0);
        assert!(p.moving());
        assert_eq!(p.fraction(10.0), Some(0.4));
        let half = p.fraction(11.0).unwrap();
        assert!((half - (0.4 + 0.6 * 0.45)).abs() < 1e-9, "{half}");
        let at_eta = p.fraction(12.0).unwrap();
        assert!((at_eta - (0.4 + 0.6 * 0.9)).abs() < 1e-9, "{at_eta}");
        // Later it still moves, slower, and stays under 1.
        let late = p.fraction(14.0).unwrap();
        let later = p.fraction(20.0).unwrap();
        assert!(
            at_eta < late && late < later && later < 1.0,
            "{late} {later}"
        );
        // The same line read again does not restart it.
        p.note(Some(0.4), Some(2.0), 13.0);
        assert_eq!(p.fraction(12.0), Some(at_eta));
        // A new line does.
        p.note(Some(0.8), Some(0.5), 13.0);
        assert_eq!(p.fraction(13.0), Some(0.8));
    }

    #[test]
    fn without_an_estimate_the_share_is_shown_as_sent() {
        let mut p = FinishProgress::default();
        p.note(Some(0.3), None, 1.0);
        assert!(!p.moving());
        assert_eq!(p.fraction(5.0), Some(0.3));
    }
}
