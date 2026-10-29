//! MO-40, the final pass sets the type. While the daemon finishes, a soft
//! front follows the finishing fraction through the transcript: ahead of it
//! the words are spoken ink (lighter, a little loose), behind it set type.
//! When the final text comes the front runs once more, quickly, in reading
//! order, and each word the final text changes changes as the front reaches
//! it, with a 2 px nod and a hairline bracket; new punctuation pops in and a
//! new capital grows (`vo_scene::final_pass`).
//!
//! The front never runs backwards and never runs ahead of the fraction the
//! daemon reports. Every look gets it: `Popup::view` runs `set_type` on each
//! live text of the transcript. `--motion set-type-off` turns it off;
//! reduced motion keeps the ink and the brackets and drops the motion.

use super::*;
use vo::scene::final_pass::FinalPass;

/// How fast the front catches up with the daemon's fraction.
const FOLLOW_SECS: f64 = 0.12;
/// The correction sweep after the final text: this long per word, within
/// the bounds below.
const SWEEP_PER_WORD: f64 = 0.030;
const SWEEP_MIN: f64 = 0.25;
const SWEEP_MAX: f64 = 0.60;
const TICK_HZ: f64 = 60.0;
/// Spoken ink: opacity and extra tracking in em per glyph. 0.018 em keeps
/// even narrow words under the envelope's 5 % tracking change.
const SPOKEN_INK: f32 = 0.72;
const SPOKEN_LOOSE: f32 = 0.018;

#[derive(Default)]
pub(crate) struct State {
    /// Where the front is during finishing, 0 to 1, and when it was moved.
    front: f32,
    front_t: f64,
    /// The live words just before the final text came.
    live: Vec<Word>,
    /// When the final text came, while the correction sweep runs or has run.
    sweep_at: Option<f64>,
    /// The sweep's length for this text.
    sweep_secs: f64,
}

/// Where a pass is: finishing with the front at `.0`, or sweeping.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Finishing(f32),
    Sweep(f32),
}

impl Popup {
    fn set_type_on(&self) -> bool {
        !self.look.on("set-type-off")
    }

    fn set_type_reduced(&self) -> bool {
        self.look.on("reduced-motion")
    }

    /// Brings the front and the live-word snapshot up to date after a
    /// message.
    pub(crate) fn sync_set_type(&mut self, t: f64) {
        if self.files.recording {
            self.set_type = State::default();
        }
        let x = &self.extra;
        match (x.finish_at, x.final_at) {
            (_, Some(at)) => {
                if self.set_type.sweep_at != Some(at) {
                    let n = self.shown.len() as f64;
                    self.set_type.sweep_secs = (n * SWEEP_PER_WORD).clamp(SWEEP_MIN, SWEEP_MAX);
                    self.set_type.sweep_at = Some(at);
                }
            }
            (Some(_), None) => {
                // The front follows the fraction, eased, never back.
                let target = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                let s = &mut self.set_type;
                let dt = (t - s.front_t).max(0.0);
                let k = if self.look.on("reduced-motion") { 1.0 } else { 1.0 - (-dt / FOLLOW_SECS).exp() };
                s.front = (s.front + (target - s.front) * k as f32).max(s.front).min(target.max(s.front));
                s.front_t = t;
                s.live = self.shown.clone();
            }
            (None, None) => {
                self.set_type.live = self.shown.clone();
                self.set_type.front_t = t;
            }
        }
    }

    fn set_type_phase(&self, t: f64) -> Option<Phase> {
        let x = &self.extra;
        if !self.set_type_on() {
            return None;
        }
        match (x.finish_at, self.set_type.sweep_at) {
            (_, Some(at)) => {
                let k = if self.set_type_reduced() {
                    1.0
                } else {
                    ((t - at) / self.set_type.sweep_secs.max(1e-3)).clamp(0.0, 1.0)
                };
                Some(Phase::Sweep(k as f32))
            }
            (Some(_), None) => {
                let s = &self.set_type;
                let target = self.files.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                let dt = (t - s.front_t).max(0.0);
                let k = if self.set_type_reduced() { 1.0 } else { 1.0 - (-dt / FOLLOW_SECS).exp() };
                Some(Phase::Finishing((s.front + (target - s.front).max(0.0) * k as f32).min(1.0)))
            }
            (None, None) => None,
        }
    }

    /// Keeps a frame tick armed while the front moves.
    pub(crate) fn arm_set_type_tick(&mut self, cx: &mut Cx<Msg>) {
        let t = cx.t();
        let moving = match self.set_type_phase(t) {
            Some(Phase::Finishing(at)) => {
                let target = self.files.fraction.unwrap_or(0.0) as f32;
                (target - at).abs() > 1e-3
            }
            Some(Phase::Sweep(k)) => k < 1.0,
            None => false,
        };
        if moving && self.extra.tick.is_none() {
            self.extra.tick = Some(cx.after(1.0 / TICK_HZ, Msg::Tick));
        }
    }

    /// The final pass for one live text of the transcript, or the element
    /// as it was when it is not one (a status line) or nothing is
    /// finishing.
    pub(crate) fn set_type(&self, e: Element<Msg>) -> Element<Msg> {
        let Some(phase) = self.set_type_phase(self.extra.now) else {
            return e;
        };
        let Some(words) = e.live_words().map(<[Word]>::to_vec) else {
            return e;
        };
        // Only the transcript: every word is a shown word, same id and text.
        let ours = !words.is_empty()
            && words
                .iter()
                .all(|w| self.shown.iter().any(|s| s.id == w.id && s.text == w.text));
        if !ours {
            return e;
        }
        let (global, sweep) = match phase {
            Phase::Finishing(at) => (at, false),
            Phase::Sweep(k) => (k, true),
        };
        // This node's share of the transcript, by characters, so a look
        // that puts each line in its own node reads them in turn.
        let len = |w: &[Word]| w.iter().map(|w| w.text.chars().count() + 1).sum::<usize>() as f32;
        let total = len(&self.shown).max(1.0);
        let first = self.shown.iter().position(|s| s.id == words[0].id).unwrap_or(0);
        let start = len(&self.shown[..first]) / total;
        let share = len(&words) / total;
        let at = ((global - start) / share.max(1e-3)).clamp(0.0, 1.0);
        let reduced = self.set_type_reduced();
        let light = self.look.background.r + self.look.background.g + self.look.background.b > 1.5;
        let mut pass = FinalPass::new(at)
            .spoken(SPOKEN_INK, SPOKEN_LOOSE)
            .line(if reduced { Color::TRANSPARENT } else { self.look.accent.fade(0.85) })
            .lift(if light { 0.0 } else { 0.25 })
            .bracket(self.look.accent.fade(0.6))
            .still(reduced);
        if sweep {
            // The type is set already: only the corrections play, with a
            // fainter front.
            pass = pass.spoken(1.0, 0.0).line(if reduced || at >= 1.0 {
                Color::TRANSPARENT
            } else {
                self.look.accent.fade(0.6)
            });
        } else if at >= 1.0 {
            pass = pass.line(Color::TRANSPARENT);
        }
        let e = e.final_pass(pass);
        if !sweep {
            return e;
        }
        // The live words this node showed before the final text: the span
        // of the snapshot between this node's first and last known ids.
        let live = &self.set_type.live;
        let pos = |id| live.iter().position(|w: &Word| w.id == id);
        let ids: Vec<usize> = words.iter().filter_map(|w| pos(w.id)).collect();
        let (Some(&lo), Some(&hi)) = (ids.iter().min(), ids.iter().max()) else {
            return e;
        };
        e.final_from(&live[lo..=hi])
    }
}
