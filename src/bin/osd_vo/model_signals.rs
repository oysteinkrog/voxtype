//! The live model's own signals on the live text (ASR-05), from
//! `preview-signals.json` (`voxtype::osd::live::ModelSignals`), each behind a
//! motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`). All are off unless
//! asked for.
//!
//! - `ghost-tail`: the words the model hears in its lookahead (the newest
//!   0.56 s, decoded but not committed) follow the live text at 45 % ink,
//!   where the committed words will land. When a word commits, the real word
//!   takes its place and the ghost fades; when the guess changes, the old
//!   ghost fades out as the new one fades in.
//! - `three-tier-ink`: the tail in two tiers below the committed text's full
//!   ink. A pending word, 3 or more frames from the window edge, is about
//!   99 % right and sits at 70 % ink; an edge guess, closer than that, is
//!   right only 17 to 38 % of the time and sits at 30 %.
//! - `listening-floor`: a thin ribbon under the text, one bar per 80 ms
//!   frame of the last 5.6 s, its height log10(1 - P(blank)) clipped to
//!   [-6, 0]: flat in silence, rising where the model nearly heard a token.
//!   The bars ease to each step's values and slide as new frames come.
//!
//! Nothing shows once the recording stops, so the signals end with the
//! dictation and nothing moves at room tone. Under reduced motion the ghosts
//! swap at once (no fades) and the floor jumps between steps: the ink tiers
//! stay.
//!
//! A look takes part by passing its text column through
//! [`Popup::signal_marks`].

use super::*;

use voxtype::osd::live::{ModelSignals, TailToken};

pub(crate) const GHOST_TAIL: &str = "ghost-tail";
pub(crate) const THREE_TIER: &str = "three-tier-ink";
pub(crate) const FLOOR: &str = "listening-floor";

/// Ink of a tail word with `ghost-tail` alone.
const GHOST_INK: f32 = 0.45;
/// Ink of a pending tail word (`three-tier-ink`).
const PENDING_INK: f32 = 0.7;
/// Ink of an edge guess (`three-tier-ink`).
const EDGE_INK: f32 = 0.3;
/// A tail token this many frames or more from the window edge is pending.
const PENDING_FRAMES: u32 = 3;
/// The floor's range in log10(1 - P(blank)).
const FLOOR_MIN: f32 = -6.0;
/// The floor's tallest bar and its resting height, in px.
const FLOOR_PX: f32 = 4.0;
const FLOOR_REST_PX: f32 = 0.5;

/// One word of the tail.
#[derive(Debug, PartialEq)]
struct TailWord {
    text: String,
    /// Frames between its newest token and the window edge.
    edge: u32,
    /// It continues the last committed word ("act" + "ivity").
    joins: bool,
}

/// Groups the tail's tokens into words. A token that starts with a space
/// starts a word; a first token without one continues the last committed
/// word, when there is one.
fn tail_words(tail: &[TailToken], has_committed: bool) -> Vec<TailWord> {
    let mut words: Vec<TailWord> = Vec::new();
    for (i, t) in tail.iter().enumerate() {
        let starts = t.text.starts_with(' ');
        let piece = t.text.trim_start();
        if piece.is_empty() {
            continue;
        }
        match words.last_mut() {
            Some(w) if !starts => {
                w.text.push_str(piece);
                w.edge = w.edge.min(t.edge);
            }
            _ => words.push(TailWord {
                text: piece.to_string(),
                edge: t.edge,
                joins: i == 0 && !starts && has_committed,
            }),
        }
    }
    words
}

impl Popup {
    fn signals_on(&self) -> bool {
        [GHOST_TAIL, THREE_TIER, FLOOR].iter().any(|f| self.look.on(f))
    }

    fn signals_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// The signals to draw now: while the recording runs and before the
    /// final pass.
    fn live_signals(&self) -> Option<&ModelSignals> {
        let listening =
            self.files.recording && !self.status().finishing && self.ending().is_none();
        self.model_signals.as_ref().filter(|_| listening)
    }

    /// The look's text column with the model signals its features ask for:
    /// the ghost tail after `words` (the live text as laid out at `width`)
    /// and the listening floor under it. `origin` is where the live text
    /// starts inside `text`; `ink` is the live text's colour.
    pub(crate) fn signal_marks(
        &self,
        text: Element<Msg>,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Element<Msg> {
        if !self.signals_on() {
            return text;
        }
        let mut text = text;
        if let Some(s) = self.live_signals() {
            for e in self.ghosts(s, words, width, origin, ink) {
                text = text.child(e);
            }
            for e in self.floor(s, width, origin) {
                text = text.child(e);
            }
        }
        text
    }

    fn ghosts(
        &self,
        s: &ModelSignals,
        words: &[Word],
        width: f32,
        origin: (f32, f32),
        ink: Color,
    ) -> Vec<Element<Msg>> {
        let l = &self.look;
        let tiers = l.on(THREE_TIER);
        if !(l.on(GHOST_TAIL) || tiers) || s.tail.is_empty() {
            return Vec::new();
        }
        // The tail belongs to the text on screen only if that text is where
        // the model's text starts; otherwise one of the two files is newer.
        let committed = self.files.preview.trim();
        if !s.with_tail.starts_with(committed) {
            return Vec::new();
        }
        let style = l.text_style();
        let line_h = style.line_height;
        let mut full = words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let tail = tail_words(&s.tail, !words.is_empty());
        let mut ranges = Vec::with_capacity(tail.len());
        for w in &tail {
            if !w.joins && !full.is_empty() {
                full.push(' ');
            }
            let start = full.len();
            full.push_str(&w.text);
            ranges.push(start..full.len());
        }
        let laid = self.measure.layout(&full, &style, Some(width));
        let reduced = self.signals_reduced();
        let presence = if reduced {
            (Transition::NONE, Transition::NONE)
        } else {
            (Transition::fade(), Transition::fade())
        };
        // A ghost's key is the word's place in the dictation and its text:
        // the same guess keeps its element, a new guess at that place
        // fades in over the old one.
        let first = self.cut + words.len();
        let mut out = Vec::new();
        for (i, (w, range)) in tail.iter().zip(&ranges).enumerate() {
            let Some(g) = laid
                .glyphs
                .iter()
                .find(|g| range.contains(&(g.cluster as usize)))
            else {
                continue;
            };
            let top = (g.pos.y / line_h).floor() * line_h;
            // A guess that would open a line the text has no room for yet
            // waits until the text gets there.
            if top >= self.lines as f32 * line_h - 0.5 {
                continue;
            }
            let alpha = match (tiers, w.edge >= PENDING_FRAMES) {
                (true, true) => PENDING_INK,
                (true, false) => EDGE_INK,
                (false, _) => GHOST_INK,
            };
            let place = first + i - usize::from(w.joins);
            let ghost = Element::label(format!("ghost-{place}-{}", w.text), w.text.as_str())
                .text_style(style.clone())
                .color(ink)
                .opacity(alpha)
                .motion(
                    Prop::Opacity,
                    if reduced { Spring::SNAP } else { Spring::critical_to_98(0.2) },
                )
                .motion(Prop::Position, Motion::Snap)
                .presence(presence.0, presence.1);
            out.push(kit::at(ghost, origin.0 + g.pos.x, origin.1 + top));
        }
        out
    }

    fn floor(&self, s: &ModelSignals, width: f32, origin: (f32, f32)) -> Vec<Element<Msg>> {
        let l = &self.look;
        if !l.on(FLOOR) || s.frames.is_empty() {
            return Vec::new();
        }
        let slots = voxtype::osd::live::SIGNALS_KEEP_FRAMES;
        let bar = width / slots as f32;
        let base = origin.1 + self.lines as f32 * l.text_style().line_height - 1.0;
        let spring = if self.signals_reduced() {
            Spring::SNAP
        } else {
            Spring::critical_to_98(0.3)
        };
        // The newest frame sits at the right end; older ones slide left.
        let first = s.frames_total as usize - s.frames.len();
        let lead = slots - s.frames.len();
        s.frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let v = ((f.level.clamp(FLOOR_MIN, 0.0) - FLOOR_MIN) / -FLOOR_MIN).powf(1.5);
                let h = FLOOR_REST_PX + (FLOOR_PX - FLOOR_REST_PX) * v;
                let x = origin.0 + (lead + i) as f32 * bar;
                let e = Element::shape(format!("floor-{}", first + i), ShapeKind::RRect)
                    .size((bar - 1.0).max(1.0), h)
                    .fill(l.accent.fade(0.35 + 0.4 * v))
                    .motion(Prop::Size, spring)
                    .motion(Prop::Position, spring)
                    .presence(Transition::fade(), Transition::fade());
                kit::at(e, x, base - h)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, edge: u32) -> TailToken {
        TailToken {
            text: text.into(),
            start: 0.0,
            end: 0.08,
            edge,
            conf: 0.9,
            detail: None,
        }
    }

    #[test]
    fn tail_tokens_group_into_words_with_their_nearest_edge() {
        let tail = [tok(" of", 5), tok(" v", 4), tok("o", 3), tok("ice", 3), tok(" act", 1), tok("ing", 0)];
        let words = tail_words(&tail, true);
        let got: Vec<(&str, u32, bool)> = words.iter().map(|w| (w.text.as_str(), w.edge, w.joins)).collect();
        assert_eq!(got, [("of", 5, false), ("voice", 3, false), ("acting", 0, false)]);
    }

    #[test]
    fn a_first_token_without_a_space_continues_the_last_word() {
        let words = tail_words(&[tok("ivity", 4), tok(" det", 1)], true);
        assert!(words[0].joins);
        assert!(!words[1].joins);
        // With nothing committed it starts the text.
        assert!(!tail_words(&[tok("Th", 7)], false)[0].joins);
    }
}
