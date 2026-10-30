//! `forming-cue` (LAT-5): a word-shaped placeholder at the insertion point
//! while you speak and no word or guess covers the speech yet.
//!
//! The model shows a new guess at most every step, so the cue does not wait
//! for it: it reads the live voice level. It shows in the first view after
//! the voice starts, after the last committed word (or after the ghost tail
//! when there is one), and grows about one letter per 80 ms of speech that
//! no word explains yet, up to about a long word. It pulses with the level,
//! holds still in short dips, and fades out over 150 ms of quiet. When
//! words or guesses arrive they explain the speech up to their end time, so
//! the cue shrinks to what is left or goes. When the newest model frame says
//! a token is near (log10 of its non-blank probability above -2.5) it
//! brightens a little; that is a bonus, never the trigger. Each look draws
//! it in its own colour, as a bar or as dots, always fainter than the text.
//! Reduced motion: a still placeholder of fixed size that appears and goes.
//!
//! On by default in every look's set (`defaults.rs`); `no-forming-cue`
//! turns it off. While it is on, `pre-echo` draws no blobs, since both mark
//! the same place.

use super::*;

pub(crate) const FORMING: &str = "forming-cue";

/// The cue fades out over this much quiet.
pub(crate) const FADE_SECS: f64 = 0.15;
/// About one letter per this much unexplained speech.
const LETTER_SECS: f64 = 0.08;
/// The cue's longest, in letters.
const MAX_LETTERS: f32 = 11.0;
/// Its shortest, in letters: the size it shows at on the first frame.
const MIN_LETTERS: f32 = 1.5;
/// Under reduced motion, its one size.
const STILL_LETTERS: f32 = 3.0;
/// After the text's last line moves (a wrap, a scroll), the cue waits this
/// long: the lines spring to their place and would pass through it.
const LINE_SETTLE_SECS: f64 = 0.35;
/// How long a word takes to land in the looks whose words move in.
const LAND_SECS: f64 = 0.25;
/// How long a loading line where the words go takes to fade out once the
/// first word comes.
const HINT_FADE_SECS: f64 = 0.3;
/// This soon after a voice onset the cue shows whatever the words do.
const ONSET_SECS: f64 = 0.05;
/// The model's frame signal above this says a token is near.
const NEAR_LEVEL: f32 = -2.5;
/// Ink, from quiet to loud, and the bonus when a token is near.
const INK_LOW: f32 = 0.16;
const INK_HIGH: f32 = 0.38;
const INK_NEAR: f32 = 0.12;

/// The voice as the cue reads it, fed frame by frame with the hearing state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Voice {
    voiced: bool,
    /// When the speech the cue stands for began, on the audio clock.
    from: Option<f64>,
    quiet_from: Option<f64>,
    /// The newest frame's clock and level, 0 to 1.
    clock: f64,
    level: f32,
    /// The words on screen explain the audio up to here.
    explained: f64,
    /// Where the cue last stood (x, line top) and since when it stands on
    /// that line without a step back, in scene time.
    line: std::cell::Cell<Option<(f32, f32, f64)>>,
    /// The shown words' count and a hash of the last one's text, and when
    /// they last changed, in scene time.
    words: std::cell::Cell<Option<(usize, u64, f64)>>,
    /// In looks whose words land with a move: when the words last changed,
    /// how far right of its place the cue stood then, and the place it
    /// last had (x, line top).
    land: std::cell::Cell<Option<(f64, f32, f32, f32)>>,
    /// The right edge of a loading line on the text's first line, in the
    /// space of the next cue drawn: a look that writes one where the words
    /// go sets it just before, and the cue stands after it.
    pub(crate) hint: std::cell::Cell<Option<f32>>,
    /// How wide a look's loading line was, for the hand-over to the words.
    pub(crate) hint_w: std::cell::Cell<Option<f32>>,
}

impl Voice {
    /// One audio frame, with the hearing thresholds `voice` and `quiet`.
    pub(crate) fn hear(&mut self, clock: f64, level: f32, voice: f32, quiet: f32) {
        self.clock = clock;
        self.level = level;
        if level >= voice {
            if !self.voiced && self.from.is_none() {
                self.from = Some(clock);
            }
            self.voiced = true;
            self.quiet_from = None;
        } else if level < quiet && self.voiced {
            self.voiced = false;
            self.quiet_from = Some(clock);
        }
        if self.quiet() > FADE_SECS {
            self.from = None;
        }
    }

    /// Words now explain the audio up to `until` seconds.
    pub(crate) fn explain(&mut self, until: f64) {
        self.explained = self.explained.max(until);
    }

    fn quiet(&self) -> f64 {
        self.quiet_from.map_or(0.0, |q| (self.clock - q).max(0.0))
    }

    /// Whether the newest frame is voice.
    pub(crate) fn voiced(&self) -> bool {
        self.voiced
    }

    /// Whether speech the cue stands for is going on now.
    pub(crate) fn active(&self) -> bool {
        self.from.is_some()
    }
}

/// How a look draws the cue.
#[derive(Clone, Copy)]
enum Pen {
    Bar,
    Dots,
}

/// The look's own colour for the cue, or the text ink.
fn material(theme: OsdTheme, ink: Color) -> (Color, Pen) {
    use OsdTheme::*;
    let c = |rgb: u32| Color::hex(rgb, 1.0);
    match theme {
        Amber => (c(0xff9a2e), Pen::Bar),
        Radial => (c(0xb8ceff), Pen::Bar),
        Tron => (c(0x29d3ff), Pen::Bar),
        Mission => (c(0xff7a1a), Pen::Bar),
        Palette => (c(0x7246fc), Pen::Bar),
        Dotmatrix => (c(0xe6e8e6), Pen::Dots),
        Swissfi => (c(0xff5a0a), Pen::Bar),
        Scope => (c(0x3dff8f), Pen::Bar),
        Captions => (c(0x6f88ff), Pen::Bar),
        Karaoke => (c(0x19a7ff), Pen::Bar),
        Island => (c(0x22b8ff), Pen::Bar),
        Jarvis => (c(0x35e6ff), Pen::Bar),
        Particles => (c(0x2fc8ff), Pen::Dots),
        Synthwave => (c(0x9b5cff), Pen::Bar),
        Mecha => (c(0xffb81c), Pen::Bar),
        Cyberpunk => (c(0xfcea2e), Pen::Bar),
        _ => (ink, Pen::Bar),
    }
}

impl Popup {
    /// Whether the cue may show now: on, and the dictation is listening (or
    /// its model is still loading: the words will come).
    pub(crate) fn forming_on(&self) -> bool {
        self.look.on(FORMING)
            && matches!(self.kit_mode(), kit::Mode::Listening | kit::Mode::Loading)
    }

    /// Seconds of speech that no word or guess explains yet, while the cue
    /// shows; `tail_end` is where the ghost tail's audio ends.
    fn forming_spoken(&self, tail_end: Option<f64>) -> Option<f64> {
        let v = &self.hearing.voice;
        let from = v.from?;
        let explained = tail_end.map_or(v.explained, |t| v.explained.max(t));
        let spoken_to = v.clock - v.quiet();
        let spoken = spoken_to - from.max(explained);
        // At the onset itself nothing is spoken yet, and the cue shows at
        // its smallest; speech that words already cover shows none.
        (spoken >= 0.0 || from >= explained).then_some(spoken.max(0.0))
    }

    /// The cue after the text, or None. `end` is where the next glyph after
    /// the shown text and tail would go (x, line top) in the text's own
    /// space, `rows` how many lines the text has room for.
    pub(crate) fn forming_mark(
        &self,
        words: &[Word],
        end: (f32, f32),
        tail_end: Option<f64>,
        text: &model_signals::TailText,
        origin: (f32, f32),
        ink: Color,
    ) -> Option<Element<Msg>> {
        if !self.forming_on() {
            return None;
        }
        let hint = self.hearing.voice.hint.take();
        let spoken = self.forming_spoken(tail_end)?;
        let now = self.extra.now;
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            words.last().map(|w| w.text.as_str()).hash(&mut h);
            (words.len(), h.finish())
        };
        let v = &self.hearing.voice;
        // Looks with a loading line where the words go: the cue stands after
        // that line while it shows and while it fades out under the first
        // words. While the line shows, a look that does not say where it
        // ends gets no cue.
        let hinted = matches!(self.look.theme, OsdTheme::Palette | OsdTheme::Jarvis);
        let onset = v.from.is_some_and(|f| v.clock - f < ONSET_SECS);
        let mut changed = false;
        let hand = match v.words.get() {
            Some((n, h, since)) if (n, h) == sig => now - since < HINT_FADE_SECS,
            last => {
                changed = true;
                let first = last.is_none_or(|(n, _, _)| n == 0) && sig.0 > 0;
                // The fade runs on through later changes.
                let fading = last
                    .filter(|_| hinted)
                    .map(|l| l.2)
                    .filter(|t| now - t < HINT_FADE_SECS);
                let since = if hinted && first {
                    now
                } else {
                    fading.unwrap_or(f64::NEG_INFINITY)
                };
                v.words.set(Some((sig.0, sig.1, since)));
                since > f64::NEG_INFINITY
            }
        };
        let hint = if hand || words.is_empty() { hint } else { None };
        if hinted && hint.is_none() && words.is_empty() && self.kit_mode() == kit::Mode::Loading {
            return None;
        }
        // Looks whose words land with a move (letters flying in, a word
        // sliding in, a centred line gliding to its new middle) cross the
        // cue's spot: while a word lands the cue stands clear by the move's
        // reach, closing in as it settles.
        let still = self.look.on("reduced-motion") || kit::reduced();
        let reach = match self.look.theme {
            _ if still => None,
            OsdTheme::Particles => Some(text.style.size * 0.8),
            OsdTheme::Mecha => Some(text.style.size * 0.6),
            OsdTheme::Synthwave => Some(text.style.size * 0.8),
            _ => None,
        };
        let landing = reach.map(|reach| {
            // The end in the look's space: a centred line moves its origin.
            let at = origin.0 + end.0;
            let (since, push, last_x, last_top) =
                v.land.get().unwrap_or((f64::NEG_INFINITY, 0.0, at, end.1));
            let (since, push) = if changed {
                // A centred line's end moves by half the width change, and
                // its glyphs start that far right of their new place.
                let slide = if (last_top - end.1).abs() <= 0.5 {
                    (at - last_x).max(0.0)
                } else {
                    0.0
                };
                (now, reach + slide)
            } else {
                (since, push)
            };
            v.land.set(Some((since, push, at, end.1)));
            push * (1.0 - ((now - since) / LAND_SECS) as f32).clamp(0.0, 1.0)
        });
        let style = &text.style;
        let letter = style.size * 0.5;
        let letters = if still {
            STILL_LETTERS
        } else {
            ((spoken / LETTER_SECS) as f32).clamp(MIN_LETTERS, MAX_LETTERS)
        };
        let gap = style.size * 0.3;
        let (mut x, mut top) = (end.0 + if end.0 > 0.0 { gap } else { 0.0 }, end.1);
        let line_h = style.line_height;
        let min_w = letter * MIN_LETTERS;
        if x + min_w > text.width {
            if top + 2.0 * line_h <= text.rows as f32 * line_h + 0.5 {
                // No room on this line: the next one, when the text has it.
                (x, top) = (0.0, top + line_h);
            } else {
                // The last line is full and the next word scrolls the text:
                // what is left of it, down to a dot at the edge.
                if x > text.width {
                    return None;
                }
            }
        }
        // An end that moves up or steps back along its line (a revision,
        // earlier lines cut) means the lines are moving to a new layout. A
        // wrap down to a new line moves nothing that is already there.
        match v.line.get() {
            None => v.line.set(Some((x, top, f64::NEG_INFINITY))),
            // Right at an onset the text has stood still through the pause,
            // so a new place is already settled, unless a word lands now.
            Some((lx, lt, _)) if top + 0.5 < lt || ((lt - top).abs() <= 0.5 && x + 0.5 < lx) => {
                let settled = onset && !changed;
                v.line.set(Some((
                    x,
                    top,
                    if settled { f64::NEG_INFINITY } else { now },
                )));
                if !settled {
                    return None;
                }
            }
            Some((_, _, since)) => {
                v.line.set(Some((x, top, since)));
                if now - since < LINE_SETTLE_SECS {
                    return None;
                }
            }
        }
        if let Some(push) = landing {
            x += push;
        }
        if let Some(h) = hint.filter(|_| top < line_h * 0.5) {
            x = x.max(h + gap);
        }
        if x + letter * 0.4 > text.width + 0.5 {
            return None;
        }
        let w = (letters * letter).min(text.width - x).max(letter * 0.4);
        let near = self
            .model_signals
            .as_ref()
            .and_then(|s| s.frames.last())
            .is_some_and(|f| f.level > NEAR_LEVEL);
        let level = if still {
            0.5
        } else {
            v.level.clamp(0.0, 1.0).sqrt()
        };
        // Reduced motion: the cue appears and goes, with no fade.
        let fade = if still {
            1.0
        } else {
            1.0 - (v.quiet() / FADE_SECS).clamp(0.0, 1.0) as f32
        };
        let alpha =
            (INK_LOW + (INK_HIGH - INK_LOW) * level + if near { INK_NEAR } else { 0.0 }) * fade;
        let (color, pen, alpha) = if probe() {
            (PROBE_INK, Pen::Bar, 1.0)
        } else {
            let (c, p) = material(self.look.theme, ink);
            (c, p, alpha)
        };
        let xh = self.forming_x_height(style);
        let base = top + self.forming_baseline(style);
        let h = (xh * 0.5).max(2.0);
        let y = base - xh * 0.5 - h * 0.5;
        let e = match pen {
            Pen::Bar => Element::shape("forming-cue", ShapeKind::RRect)
                .size(w, h)
                .radius(h * 0.5)
                .fill(color.fade(alpha)),
            Pen::Dots => {
                let r = (h * 0.45).max(1.2);
                let pitch = r * 3.2;
                let n = ((w / pitch).floor() as usize).max(1);
                let mut row = Element::layer("forming-cue").size(w, h);
                for i in 0..n {
                    row = row.child(kit::at(
                        Element::shape(format!("forming-dot-{i}"), ShapeKind::RRect)
                            .size(2.0 * r, 2.0 * r)
                            .radius(r)
                            .fill(color.fade(alpha))
                            .presence(Transition::NONE, Transition::NONE),
                        i as f32 * pitch,
                        h * 0.5 - r,
                    ));
                }
                row
            }
        };
        let e = e
            .motion(Prop::Size, Motion::Snap)
            .motion(Prop::Position, Motion::Snap)
            .motion(Prop::Opacity, Motion::Snap)
            .presence(Transition::NONE, Transition::NONE);
        Some(kit::at(e, origin.0 + x, origin.1 + y))
    }

    /// The baseline of a line of `style`, below its top.
    fn forming_baseline(&self, style: &TextStyle) -> f32 {
        self.measure
            .layout("x", style, None)
            .glyphs
            .first()
            .map_or(style.line_height * 0.75, |g| g.pos.y)
    }

    fn forming_x_height(&self, style: &TextStyle) -> f32 {
        style.size * 0.5
    }
}

static PROBE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Scene times of the voice onsets (true) and the quiets after voice
/// (false) the cue saw, while probing.
static ONSETS: std::sync::Mutex<Vec<(f64, bool)>> = std::sync::Mutex::new(Vec::new());

/// `--forming-latency`: the cue in one exact colour at full ink, and each
/// voice onset noted.
pub(crate) fn probe() -> bool {
    PROBE.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn start_probe() {
    PROBE.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn note_voice(now: f64, onset: bool) {
    if probe() {
        if let Ok(mut v) = ONSETS.lock() {
            v.push((now, onset));
        }
    }
}

pub(crate) fn take_onsets() -> Vec<(f64, bool)> {
    ONSETS
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// The probe's colour: no look draws this.
pub(crate) const PROBE_INK: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
