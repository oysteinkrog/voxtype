//! What the popup heard and when each word came, for the looks that tie a
//! word to the sound that made it: the onsets, each word's arrival and last
//! change, the onset that started it and the stretch of audio it came from.
//!
//! The hydraulic chain and the onset joint (mecha), the acoustic receipts,
//! the pitch margin score and the particle negative read this. Onsets come
//! from the daemon's voice features when it sends them, else from rises in
//! the level, so the moments also show with an older daemon and in the demo.

use super::*;

use std::collections::HashMap;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};
use vo::scene::path::{Contour, StrokeStyle};

use vo_core::signals::{Sig, ONSET_THRESHOLD};
use vo_core::words::WordId;

/// Onsets closer than this to the one before are dropped: at most four a
/// second reach a look, so a buzz never chatters.
const ONSET_GAP: f64 = 0.25;
/// Level rise over the quietest of the last `RISE_FRAMES` frames that counts
/// as an onset when the daemon sends no voice features.
const RISE: f32 = 0.12;
const RISE_FRAMES: usize = 8;
/// A word claims the newest unclaimed onset at most this long before it came.
const ONSET_REACH: f64 = 1.2;
/// From the end of a word's sound to its text on screen, when the engine
/// gives no word times.
const LATENCY: f64 = 0.18;
/// The longest stretch of sound one word is said to come from.
const LONGEST: f64 = 1.2;
/// Onsets kept.
const KEEP: usize = 48;

/// One word as heard: when it came and the sound it came from, all in
/// scene time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Heard {
    pub id: WordId,
    /// When the word first showed.
    pub born: f64,
    /// When its text last changed after that.
    pub changed: Option<f64>,
    /// The onset that started it, if one came shortly before.
    pub onset: Option<f64>,
    /// The sound that made it, start and end. From the engine's word times
    /// when they fit, else from arrival order.
    pub sound: (f64, f64),
    /// The voiced pitch over that sound, 0 to 1 on the signal bus's log
    /// scale; `None` without voice features or without a voiced frame.
    pub pitch: Option<f32>,
    /// The word came with engine word times that fit the audio.
    pub timed: bool,
}

/// What the popup keeps about the words and the sound.
#[derive(Default)]
pub(crate) struct State {
    words: HashMap<WordId, Heard>,
    texts: HashMap<WordId, String>,
    /// Onsets in scene time, oldest first.
    onsets: Vec<f64>,
    /// Onsets a word has claimed.
    claimed: Vec<f64>,
    /// The scene time up to which the levels were scanned for onsets.
    scanned: f64,
}

impl State {
    /// A word on screen as heard, if it was seen arriving.
    pub(crate) fn word(&self, id: WordId) -> Option<&Heard> {
        self.words.get(&id)
    }

    /// The onsets in scene time, oldest first.
    pub(crate) fn onsets(&self) -> &[f64] {
        &self.onsets
    }

    fn add_onset(&mut self, at: f64) {
        if self.onsets.last().is_some_and(|&l| at - l < ONSET_GAP) {
            return;
        }
        self.onsets.push(at);
        if self.onsets.len() > KEEP {
            self.onsets.remove(0);
        }
    }
}

/// Onsets in `levels` (one per frame, the newest shown at `newest_at`) after
/// scene time `after`: a rise of `RISE` over the quietest recent frame.
pub(crate) fn level_onsets(levels: &[f32], newest_at: f64, after: f64) -> Vec<f64> {
    let n = levels.len();
    let mut out = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for j in RISE_FRAMES..n {
        let at = newest_at - (n - 1 - j) as f64 / FRAME_HZ as f64;
        let low = levels[j - RISE_FRAMES..j]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let rising = levels[j] - low >= RISE && levels[j - 1] - low < RISE;
        if rising && at - last >= ONSET_GAP {
            last = at;
            if at > after {
                out.push(at);
            }
        }
    }
    out
}

/// A damped spring's step response from 0 to 1, `dt` seconds after the
/// push: fast, a little overshoot, still within about 150 ms. 0 before.
pub(crate) fn piston(dt: f64) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    let (w, z): (f64, f64) = (2.0 * std::f64::consts::PI * 11.0, 0.62);
    let wd = w * (1.0 - z * z).sqrt();
    let e = (-z * w * dt).exp();
    (1.0 - e * ((wd * dt).cos() + z * w / wd * (wd * dt).sin())) as f32
}

/// A knock that rises and falls back: 0 at the push, 1 at `peak` seconds,
/// back near 0 by about five times that.
pub(crate) fn knock(dt: f64, peak: f64) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    let x = dt / peak;
    (x * (1.0 - x).exp()) as f32
}

/// Where one word of the live text sits in its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WordBox {
    pub id: WordId,
    /// Ink box: left and right of its glyphs, a line's height around the
    /// baseline.
    pub rect: Rect,
    pub baseline: f32,
    pub line: usize,
}

/// The words of `words` laid out as the live text lays them out, in its
/// box's space, with the lines counted from the top of the layout.
pub(crate) fn word_boxes(
    measure: &vo::ShapedLayout,
    words: &[Word],
    style: &TextStyle,
    width: Option<f32>,
) -> (Vec<WordBox>, usize) {
    let mut text = String::new();
    let mut starts = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        starts.push(text.len() as u32);
        text.push_str(&w.text);
    }
    let shaped = measure.layout(&text, style, width);
    let mut boxes: Vec<Option<(f32, f32, usize)>> = vec![None; words.len()];
    for (li, line) in shaped.lines.iter().enumerate() {
        for g in &shaped.glyphs[line.glyphs.clone()] {
            let Some(i) = starts.partition_point(|&s| s <= g.cluster).checked_sub(1) else {
                continue;
            };
            let Some(word) = words.get(i) else {
                continue;
            };
            // The space before a word belongs to the word before it.
            if g.cluster >= starts[i] + word.text.len() as u32 {
                continue;
            }
            let (x0, x1) = (g.pos.x, g.pos.x + g.advance);
            let b = boxes[i].get_or_insert((x0, x1, li));
            b.0 = b.0.min(x0);
            b.1 = b.1.max(x1);
        }
    }
    let size = style.size;
    let out = boxes
        .iter()
        .zip(words)
        .filter_map(|(b, w)| {
            let (x0, x1, li) = (*b)?;
            let baseline = shaped.lines[li].baseline;
            Some(WordBox {
                id: w.id,
                rect: Rect::new(x0, baseline - 0.78 * size, x1 - x0, size),
                baseline,
                line: li,
            })
        })
        .collect();
    (out, shaped.lines.len())
}

impl Popup {
    /// Brings what was heard up to date: new onsets, words that came or
    /// changed, the sound each came from.
    pub(crate) fn sync_sounding(&mut self, t: f64) {
        // Onsets: the daemon's when it sends voice features, else rises in
        // the level.
        let signals = self.signals.get();
        let after = self.heard.scanned;
        let mut found: Vec<f64> = if signals.is_empty() {
            let levels: Vec<f32> = self.levels.iter().map(|&l| l as f32).collect();
            level_onsets(&levels, self.newest_at, after)
        } else {
            let playout = signals.playout();
            signals
                .frames()
                .filter(|(_, f)| f.onset >= ONSET_THRESHOLD)
                .map(|(at, _)| at + playout)
                .filter(|&at| at > after)
                .collect()
        };
        found.sort_by(f64::total_cmp);
        for at in found {
            self.heard.add_onset(at);
            self.heard.scanned = self.heard.scanned.max(at);
        }

        // Words: arrivals and changes.
        let doc = self
            .files
            .live
            .as_ref()
            .filter(|d| d.text == self.files.preview);
        let words = self.model.words();
        let mut prev_end: Option<f64> = None;
        for (i, w) in words.iter().enumerate() {
            let old = self.heard.texts.get(&w.id);
            if old == Some(&w.text) {
                prev_end = self.heard.words.get(&w.id).map(|h| h.sound.1);
                continue;
            }
            if old.is_some() {
                if let Some(h) = self.heard.words.get_mut(&w.id) {
                    h.changed = Some(t);
                }
                self.heard.texts.insert(w.id, w.text.clone());
                prev_end = self.heard.words.get(&w.id).map(|h| h.sound.1);
                continue;
            }
            self.heard.texts.insert(w.id, w.text.clone());
            let onset = self
                .heard
                .onsets
                .iter()
                .rev()
                .copied()
                .find(|&o| o <= t && t - o <= ONSET_REACH && !self.heard.claimed.contains(&o));
            if let Some(o) = onset {
                self.heard.claimed.push(o);
                if self.heard.claimed.len() > KEEP {
                    self.heard.claimed.remove(0);
                }
            }
            // The engine's times, when they place the sound before the text.
            let audio0 = self.anchor + PLAYOUT_DELAY_SECS;
            let timed = doc
                .and_then(|d| d.words.get(i))
                .and_then(|tw| Some((tw.start?, tw.end?)))
                .map(|(s, e)| (audio0 + s as f64, audio0 + e as f64))
                .filter(|&(s, e)| e > s && e <= t + 0.05 && t - e < 3.0);
            let sound = timed.unwrap_or_else(|| {
                let end = t - LATENCY;
                let start = [onset, prev_end]
                    .into_iter()
                    .flatten()
                    .fold(end - LONGEST, f64::max)
                    .min(end - 0.08);
                (start, end)
            });
            let pitch = word_pitch(&signals, sound);
            prev_end = Some(sound.1);
            self.heard.words.insert(
                w.id,
                Heard {
                    id: w.id,
                    born: t,
                    changed: None,
                    onset,
                    sound,
                    pitch,
                    timed: timed.is_some(),
                },
            );
        }
        // Forget words gone from the model.
        if self.heard.words.len() > words.len() + 64 {
            let live: std::collections::HashSet<WordId> = words.iter().map(|w| w.id).collect();
            self.heard.words.retain(|id, _| live.contains(id));
            self.heard.texts.retain(|id, _| live.contains(id));
        }
    }
}

/// Samples of the level per receipt.
pub(crate) const RECEIPT_SAMPLES: usize = 12;
/// The sound folds into a line by this long after the word came, and the line
/// into a tick by `TICK_AT`.
const FOLD_AT: f64 = 0.09;
const TICK_AT: f64 = 0.21;

/// One acoustic receipt (MO-71): the sound that made a word, under it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Receipt {
    /// When the word came.
    pub at: f64,
    /// The word's ink box, in the drawing's space.
    pub rect: Rect,
    /// The level over the word's sound, 0 to about 1.
    pub levels: [f32; RECEIPT_SAMPLES],
}

/// The acoustic receipts under the newest three words: the sound that made
/// each word folds into a short underline, then into a timestamp tick.
#[derive(Clone, Debug)]
pub(crate) struct Receipts {
    pub items: Vec<Receipt>,
    pub color: Color,
    /// Text size in pixels: the receipt's height scales with it.
    pub size: f32,
    pub reduced: bool,
}

impl Receipts {
    /// The scene keeps drawing until the newest receipt has become a tick.
    pub(crate) fn until(&self) -> f64 {
        self.items
            .iter()
            .map(|r| r.at)
            .fold(f64::NEG_INFINITY, f64::max)
            + TICK_AT
            + 0.05
    }

    /// Draws them, `origin` being the drawing space's top left on the surface.
    pub(crate) fn draw(&self, t: f64, origin: Point, p: &mut Painter) {
        let thin = (0.045 * self.size).clamp(1.0, 2.0);
        for (i, r) in self.items.iter().enumerate() {
            let dt = t - r.at;
            if dt < 0.0 {
                continue;
            }
            // Older receipts are quieter.
            let quiet = [1.0, 0.7, 0.5][i.min(2)];
            let x0 = origin.x + r.rect.x;
            let y = origin.y + r.rect.y + r.rect.h + 0.14 * self.size;
            let tick = |p: &mut Painter, a: f32| {
                p.shape(ShapeInstance::rrect(
                    RRect::new(
                        Rect::new(x0, y - 0.12 * self.size, thin, 0.2 * self.size),
                        0.0,
                    ),
                    Paint::solid(self.color.fade(a)),
                ));
            };
            if self.reduced || dt >= TICK_AT {
                tick(p, 0.55 * quiet);
                continue;
            }
            if dt < FOLD_AT {
                // The sound, as bars that flatten into the underline.
                let k = 1.0 - ease(dt / FOLD_AT);
                let n = RECEIPT_SAMPLES as f32;
                let pitch = r.rect.w / n;
                for (j, &l) in r.levels.iter().enumerate() {
                    let h = thin + 0.32 * self.size * l.clamp(0.0, 1.0).sqrt() * k;
                    let bx = x0 + j as f32 * pitch;
                    p.shape(ShapeInstance::rrect(
                        RRect::new(Rect::new(bx, y - h / 2.0, (pitch * 0.6).max(1.0), h), 0.0),
                        Paint::solid(self.color.fade(0.8 * quiet)),
                    ));
                }
                continue;
            }
            // The underline shrinks to its start: the tick.
            let k = ease((dt - FOLD_AT) / (TICK_AT - FOLD_AT));
            let w = (r.rect.w * (1.0 - k)).max(thin);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0, y - thin / 2.0, w, thin), 0.0),
                Paint::solid(self.color.fade(0.8 * quiet)),
            ));
            tick(p, 0.55 * quiet * k);
        }
    }
}

/// Up to this many words of a phrase get a pitch mark (MO-72).
pub(crate) const SCORE_MARKS: usize = 6;
/// A pause this long starts a new phrase.
const PHRASE_GAP: f64 = 1.0;
/// The smallest pitch range the marks spread over, on the bus's log scale
/// (about four semitones).
const MIN_RANGE: f32 = 0.08;
/// Seconds a mark takes to find its place, and the contour to reach it.
const MARK_SECS: f64 = 0.10;
const JOIN_SECS: f64 = 0.24;

/// The pitch margin score (MO-72): beside the reading line, up to six marks
/// place the phrase's pitch and join into a contour. Without voice features
/// there is no pitch, and no marks.
#[derive(Clone, Debug)]
pub(crate) struct Score {
    /// When each word came and its pitch, 0 low to 1 high in the speaker's
    /// recent range; oldest first.
    pub marks: Vec<(f64, f32)>,
    /// The box beside the line, in the drawing's space.
    pub rect: Rect,
    pub color: Color,
    pub reduced: bool,
}

impl Score {
    pub(crate) fn until(&self) -> f64 {
        self.marks
            .last()
            .map_or(f64::NEG_INFINITY, |m| m.0 + JOIN_SECS + 0.05)
    }

    pub(crate) fn draw(&self, t: f64, origin: Point, p: &mut Painter) {
        let r = self.rect;
        let n = SCORE_MARKS as f32;
        let step = r.w / (n - 1.0);
        let place = |i: usize, (at, y): (f64, f32)| {
            let k = if self.reduced {
                1.0
            } else {
                ease((t - at) / MARK_SECS)
            };
            let mid = 0.5;
            let yy = mid + (y - mid) * k;
            Point::new(
                origin.x + r.x + i as f32 * step,
                origin.y + r.y + (1.0 - yy) * r.h,
            )
        };
        let shown: Vec<(usize, Point, f64)> = self
            .marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.0 <= t)
            .map(|(i, &m)| (i, place(i, m), m.0))
            .collect();
        // The contour: each joint draws on once its mark has its place.
        for w in shown.windows(2) {
            let (a, b) = (w[0].1, w[1].1);
            let k = if self.reduced {
                1.0
            } else {
                ease((t - w[1].2 - MARK_SECS) / (JOIN_SECS - MARK_SECS))
            };
            if k <= 0.0 {
                continue;
            }
            let end = Point::new(a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k);
            p.stroke(
                &Contour::polyline(&[a, end], false),
                &StrokeStyle::solid(1.2, self.color.fade(0.55)),
            );
        }
        for &(_, c, at) in &shown {
            let a = if self.reduced {
                1.0
            } else {
                ease((t - at) / MARK_SECS)
            };
            p.shape(ShapeInstance::circle(
                c,
                1.8,
                Paint::solid(self.color.fade(0.9 * a)),
            ));
        }
    }
}

/// Dots in one word's negative, at most.
const NEGATIVE_DOTS: usize = 48;
/// When the negative shows after the word came (its particles have landed),
/// how long it takes to trace, and when it has folded into a tick.
const NEGATIVE_AFTER: f64 = 0.22;
const NEGATIVE_TRACE: f64 = 0.07;
const NEGATIVE_FOLD: f64 = 0.17;
/// The tick stays this long once folded.
const NEGATIVE_TICK: f64 = 0.25;

/// A word's negative impression (MO-69): the dots under a word where its
/// mirrored, flattened glyphs are not, relative to the word's left end on
/// its baseline. Cached per word text and size.
pub(crate) fn negative(
    fonts: &FontRegistry,
    measure: &vo::ShapedLayout,
    text: &str,
    style: &TextStyle,
) -> Arc<[Point]> {
    use std::sync::{Mutex, OnceLock};
    use vo::scene::path::kurbo::{Affine as KAffine, Shape};
    type Cache = HashMap<(String, u32, u32), Arc<[Point]>>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let key = (text.to_string(), style.font, (style.size * 64.0) as u32);
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(p) = cache.get(&key) {
        return p.clone();
    }
    if cache.len() > 256 {
        cache.clear();
    }
    let shaped = measure.layout(text, style, None);
    let size = style.size;
    let base = shaped.lines.first().map_or(0.8 * size, |l| l.baseline);
    let paths: Vec<_> = shaped
        .glyphs
        .iter()
        .filter_map(|g| {
            let face = fonts.get(g.font)?;
            let path = vo::scene::path::glyph_path(&face, g.glyph, size);
            Some(KAffine::translate((g.pos.x as f64, (g.pos.y - base) as f64)) * path)
        })
        .collect();
    let w = shaped.size.w.max(1.0);
    // The impression lies under the baseline, half as tall as the letters.
    let (depth, letters) = (0.36 * size, 0.72 * size);
    let rows = 4usize;
    let cols = ((NEGATIVE_DOTS * 2) / rows).max(1);
    let step = w / cols as f32;
    let mut out = Vec::new();
    for r in 0..rows {
        let d = (r as f32 + 0.5) / rows as f32;
        for c in 0..cols {
            let x = (c as f32 + 0.5 + if r % 2 == 1 { 0.5 } else { 0.0 }) * step;
            if x >= w {
                continue;
            }
            // Mirrored: a dot `d` of the way down the impression stands for
            // the point `d` of the way up the letters.
            let q = vo::scene::path::kurbo::Point::new(x as f64, -(d * letters) as f64);
            if !paths.iter().any(|p| p.contains(q)) {
                out.push(Point::new(x, 0.06 * size + d * depth));
            }
        }
    }
    // A few dots: keep every other one until at most `NEGATIVE_DOTS`.
    while out.len() > NEGATIVE_DOTS {
        out = out.into_iter().step_by(2).collect();
    }
    let out: Arc<[Point]> = out.into();
    cache.insert(key, out.clone());
    out
}

/// The negatives of the newest words: the dots trace in, then fold into a
/// baseline tick at the word's start, then the tick goes.
#[derive(Clone, Debug)]
pub(crate) struct Negatives {
    /// When each word landed, its left end on the baseline in the drawing's
    /// space, and its dots.
    pub items: Vec<(f64, Point, Arc<[Point]>)>,
    pub color: Color,
    pub size: f32,
    pub reduced: bool,
}

impl Negatives {
    pub(crate) fn until(&self) -> f64 {
        self.items
            .iter()
            .map(|i| i.0)
            .fold(f64::NEG_INFINITY, f64::max)
            + NEGATIVE_AFTER
            + NEGATIVE_FOLD
            + NEGATIVE_TICK
    }

    pub(crate) fn draw(&self, t: f64, origin: Point, p: &mut Painter) {
        let s = self.size;
        let dot = (0.06 * s).clamp(1.0, 1.8);
        for (at, o, dots) in &self.items {
            let dt = t - at - NEGATIVE_AFTER;
            let end = NEGATIVE_FOLD + NEGATIVE_TICK;
            if !(0.0..end).contains(&dt) {
                continue;
            }
            let (ox, oy) = (origin.x + o.x, origin.y + o.y);
            let tick = Rect::new(ox, oy + 0.12 * s, 0.32 * s, (0.05 * s).max(1.0));
            let tick_a = 1.0 - ((dt - NEGATIVE_FOLD) / NEGATIVE_TICK).clamp(0.0, 1.0) as f32;
            if self.reduced || dt >= NEGATIVE_FOLD {
                p.shape(ShapeInstance::rrect(
                    RRect::new(tick, 0.0),
                    Paint::solid(self.color.fade(0.8 * tick_a)),
                ));
                continue;
            }
            let shown = ease(dt / NEGATIVE_TRACE);
            let fold = ease((dt - NEGATIVE_TRACE) / (NEGATIVE_FOLD - NEGATIVE_TRACE));
            let n = dots.len().max(1) as f32;
            for (i, d) in dots.iter().enumerate() {
                // Traced left to right, then each dot runs to its place on
                // the tick.
                let a = (shown * n - i as f32).clamp(0.0, 1.0);
                let to = Point::new(tick.x + tick.w * i as f32 / n, tick.y + tick.h / 2.0);
                let x = ox + d.x + (to.x - ox - d.x) * fold;
                let y = oy + d.y + (to.y - oy - d.y) * fold;
                p.shape(ShapeInstance::circle(
                    Point::new(x, y),
                    dot,
                    Paint::solid(self.color.fade(0.75 * a)),
                ));
            }
        }
    }
}

/// MO-71: the sound that made each of the newest words folds into an
/// underline, then a timestamp tick.
pub(crate) const ACOUSTIC_RECEIPTS: &str = "acoustic-receipts";
/// MO-72: up to six marks beside the line place the phrase's pitch and join
/// into a contour.
pub(crate) const PITCH_SCORE: &str = "pitch-score";

impl Popup {
    /// Reduced motion, from the environment or the look's features.
    pub(crate) fn still_motion(&self) -> bool {
        kit::reduced() || self.look.on("reduced-motion")
    }

    /// The pitch score as an element whose box is `rect` (in its parent),
    /// when the look has the feature on.
    pub(crate) fn score_element(&self, rect: Rect, color: Color) -> Option<Element<Msg>> {
        if !self.look.on(PITCH_SCORE) {
            return None;
        }
        let pad = 4.0;
        let score = self.score(
            Rect::new(pad, pad, rect.w, rect.h),
            color,
            self.still_motion(),
        );
        let until = score.until();
        Some(
            kit::at(
                Element::canvas("pitch-score", until, move |t, r, p| {
                    score.draw(t, Point::new(r.x, r.y), p)
                }),
                rect.x - pad,
                rect.y - pad,
            )
            .size(rect.w + 2.0 * pad, rect.h + 2.0 * pad),
        )
    }

    /// The pitch score `w` wide just right of the newest line of the live
    /// text whose box is at `(x, y)`, `width` wide.
    pub(crate) fn score_beside(
        &self,
        x: f32,
        y: f32,
        width: f32,
        w: f32,
        color: Color,
    ) -> Option<Element<Msg>> {
        if !self.look.on(PITCH_SCORE) {
            return None;
        }
        let style = self.look.text_style();
        let (_, lines) = word_boxes(&self.measure, &self.shown, &style, Some(width));
        let lh = style.line_height;
        let top = y + lines.saturating_sub(1) as f32 * lh;
        self.score_element(
            Rect::new(x + width + 10.0, top + 0.2 * lh, w, 0.55 * lh),
            color,
        )
    }

    /// The acoustic receipts under `words` laid out in a box at `(x, y)` (in
    /// its parent) `width` wide, when the look has the feature on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn receipts_element(
        &self,
        key: &str,
        words: &[Word],
        style: &TextStyle,
        x: f32,
        y: f32,
        width: f32,
        color: Color,
    ) -> Option<Element<Msg>> {
        if !self.look.on(ACOUSTIC_RECEIPTS) || words.is_empty() {
            return None;
        }
        let (boxes, lines) = word_boxes(&self.measure, words, style, Some(width));
        let r = self.receipts(&boxes, 0.0, style.size, color, self.still_motion());
        let until = r.until();
        let h = lines.max(1) as f32 * style.line_height + 0.6 * style.size;
        Some(
            kit::at(
                Element::canvas(format!("receipts-{key}"), until, move |t, rr, p| {
                    r.draw(t, Point::new(rr.x, rr.y), p)
                }),
                x,
                y,
            )
            .size(width, h),
        )
    }
}

/// MO-73: an onset opens a 2 px joint in the caret's foot; the word it
/// starts gets the joint as its baseline tick for 120 ms.
pub(crate) const ONSET_JOINT: &str = "onset-joint";
/// The joint opens over this long, then closes by `JOINT_SECS`.
const JOINT_OPEN: f64 = 0.035;
const JOINT_SECS: f64 = 0.11;
/// A word's baseline tick stays this long.
const JOINT_TICK: f64 = 0.12;

/// How far the caret's foot is split at `t`, 0 to 1 (1 is the full 2 px),
/// from the newest onset at or before `t`.
pub(crate) fn joint_gap(onsets: &[f64], t: f64, reduced: bool) -> f32 {
    let Some(&at) = onsets.iter().rev().find(|&&at| at <= t) else {
        return 0.0;
    };
    let dt = t - at;
    if reduced {
        return if dt < JOINT_SECS { 1.0 } else { 0.0 };
    }
    if dt < JOINT_OPEN {
        ease(dt / JOINT_OPEN)
    } else {
        1.0 - ((dt - JOINT_OPEN) / (JOINT_SECS - JOINT_OPEN)).clamp(0.0, 1.0) as f32
    }
}

/// The caret's foot: a bar `w` wide centred on `cx`, its top at `y`, split
/// 2 px at its middle while an onset's joint is open.
pub(crate) fn draw_foot(p: &mut Painter, gap: f32, cx: f32, y: f32, w: f32, h: f32, color: Color) {
    let g = 2.0 * gap;
    let half = (w - g) / 2.0;
    for x in [cx - w / 2.0, cx + g / 2.0] {
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(x, y, half, h), 0.0),
            Paint::solid(color),
        ));
    }
}

/// The baseline ticks of words an onset started, for 120 ms after each came.
#[derive(Clone, Debug)]
pub(crate) struct JointTicks {
    pub items: Vec<(f64, Rect)>,
    pub color: Color,
    pub size: f32,
    /// A caret of its own on a foot, for looks without one: where it
    /// stands (left end on the baseline) and the onsets that split it.
    pub caret: Option<(Point, Arc<[f64]>)>,
    pub reduced: bool,
}

impl JointTicks {
    pub(crate) fn until(&self) -> f64 {
        self.items
            .iter()
            .map(|i| i.0)
            .fold(f64::NEG_INFINITY, f64::max)
            + JOINT_TICK
    }

    pub(crate) fn draw(&self, t: f64, origin: Point, p: &mut Painter) {
        for &(at, r) in &self.items {
            let dt = t - at;
            if !(0.0..JOINT_TICK).contains(&dt) {
                continue;
            }
            let a = 1.0 - (dt / JOINT_TICK) as f32;
            let w = r.w.min(0.4 * self.size);
            let tr = Rect::new(
                origin.x + r.x,
                origin.y + r.y + r.h + 0.04 * self.size,
                w,
                2.0,
            );
            p.shape(ShapeInstance::rrect(
                RRect::new(tr, 0.0),
                Paint::solid(self.color.fade(a)),
            ));
        }
        if let Some((c, onsets)) = &self.caret {
            let s = self.size;
            let (x, y) = (origin.x + c.x, origin.y + c.y);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x, y - 0.72 * s, 2.0, 0.72 * s), 0.0),
                Paint::solid(self.color),
            ));
            let gap = joint_gap(onsets, t, self.reduced);
            draw_foot(p, gap, x + 1.0, y + 0.06 * s, 0.4 * s, 2.0, self.color);
        }
    }
}

impl Popup {
    /// The onsets, when the look has the joint on.
    pub(crate) fn joint_onsets(&self) -> Option<Arc<[f64]>> {
        self.look
            .on(ONSET_JOINT)
            .then(|| self.heard.onsets().into())
    }

    /// The baseline ticks under `words` laid out in a box at `(x, y)`,
    /// `width` wide, when the look has the joint on; with `caret`, also a
    /// caret on a foot after the newest word while listening.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn joint_ticks_element(
        &self,
        key: &str,
        words: &[Word],
        style: &TextStyle,
        x: f32,
        y: f32,
        width: f32,
        color: Color,
        caret: bool,
    ) -> Option<Element<Msg>> {
        if !self.look.on(ONSET_JOINT) || words.is_empty() {
            return None;
        }
        let (boxes, lines) = word_boxes(&self.measure, words, style, Some(width));
        let items = boxes
            .iter()
            .filter_map(|b| {
                let h = self.heard.word(b.id)?;
                h.onset.map(|_| (h.born, b.rect))
            })
            .collect();
        let caret = (caret && self.ended_at.is_none() && self.extra.finish_at.is_none())
            .then(|| boxes.last())
            .flatten()
            .map(|b| {
                let at = Point::new(b.rect.x + b.rect.w + 0.15 * style.size, b.baseline);
                (at, Arc::<[f64]>::from(self.heard.onsets()))
            });
        let until = if caret.is_some() {
            self.heard
                .onsets()
                .last()
                .map_or(f64::NEG_INFINITY, |&o| o + JOINT_SECS)
        } else {
            f64::NEG_INFINITY
        };
        let ticks = JointTicks {
            items,
            color,
            size: style.size,
            caret,
            reduced: self.still_motion(),
        };
        let until = ticks.until().max(until);
        let h = lines.max(1) as f32 * style.line_height + 0.4 * style.size;
        Some(
            kit::at(
                Element::canvas(format!("joint-{key}"), until, move |t, r, p| {
                    ticks.draw(t, Point::new(r.x, r.y), p)
                }),
                x,
                y,
            )
            .size(width, h),
        )
    }
}

fn ease(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

impl Popup {
    /// Receipts under the newest three words of `boxes` (in the order of
    /// `self.shown`), offset by `dy` into the drawing's space.
    pub(crate) fn receipts(
        &self,
        boxes: &[WordBox],
        dy: f32,
        size: f32,
        color: Color,
        reduced: bool,
    ) -> Receipts {
        let levels = self.kit_raw_levels();
        let items = boxes
            .iter()
            .rev()
            .filter_map(|b| {
                let h = self.heard.word(b.id)?;
                let mut out = [0.0; RECEIPT_SAMPLES];
                let span = (h.sound.1 - h.sound.0).max(0.05);
                for (j, v) in out.iter_mut().enumerate() {
                    let s = h.sound.0 + span * (j as f64 + 0.5) / RECEIPT_SAMPLES as f64;
                    *v = levels.at(s);
                }
                Some(Receipt {
                    at: h.born,
                    rect: Rect::new(b.rect.x, b.rect.y + dy, b.rect.w, b.rect.h),
                    levels: out,
                })
            })
            .take(3)
            .collect();
        Receipts {
            items,
            color,
            size,
            reduced,
        }
    }

    /// The pitch score of the newest phrase, in `rect`.
    pub(crate) fn score(&self, rect: Rect, color: Color, reduced: bool) -> Score {
        let words = self.model.words();
        let heard: Vec<&Heard> = words.iter().filter_map(|w| self.heard.word(w.id)).collect();
        // The speaker's recent range.
        let recent: Vec<f32> = heard
            .iter()
            .rev()
            .take(24)
            .filter_map(|h| h.pitch)
            .collect();
        let lo = recent.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = recent.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let (lo, span) = if hi - lo < MIN_RANGE {
            ((lo + hi) / 2.0 - MIN_RANGE / 2.0, MIN_RANGE)
        } else {
            (lo, hi - lo)
        };
        // The phrase: back from the newest word to a pause.
        let mut phrase: Vec<(f64, f32)> = Vec::new();
        let mut next: Option<f64> = None;
        for h in heard.iter().rev() {
            if next.is_some_and(|n| n - h.born > PHRASE_GAP) || phrase.len() == SCORE_MARKS {
                break;
            }
            next = Some(h.born);
            if let Some(pitch) = h.pitch {
                phrase.push((h.born, ((pitch - lo) / span).clamp(0.0, 1.0)));
            }
        }
        phrase.reverse();
        Score {
            marks: phrase,
            rect,
            color,
            reduced,
        }
    }
}

/// The mean voiced pitch over `sound` (scene times), read as the bus reads
/// it at that moment: `None` with no voiced frame there.
fn word_pitch(signals: &vo_core::signals::Signals, sound: (f64, f64)) -> Option<f32> {
    if signals.is_empty() {
        return None;
    }
    let (mut sum, mut n) = (0.0f32, 0u32);
    let mut s = sound.0;
    while s <= sound.1 {
        if signals.sample(Sig::Voiced, s) > 0.5 {
            sum += signals.sample(Sig::Pitch, s);
            n += 1;
        }
        s += 0.02;
    }
    (n > 0).then(|| sum / n as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piston_settles_with_a_small_overshoot() {
        assert_eq!(piston(-0.1), 0.0);
        let peak = (1..60)
            .map(|i| piston(i as f64 * 0.005))
            .fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.12, "overshoot {peak}");
        assert!((piston(0.25) - 1.0).abs() < 0.02);
    }

    #[test]
    fn knock_returns() {
        assert!((knock(0.04, 0.04) - 1.0).abs() < 1e-6);
        assert!(knock(0.4, 0.04) < 0.01);
    }

    #[test]
    fn onsets_from_level_rises_are_spaced() {
        // Syllables every 100 ms: only one onset per 250 ms gets through.
        let levels: Vec<f32> = (0..200)
            .map(|i| if i % 10 < 4 { 0.5 } else { 0.02 })
            .collect();
        let on = level_onsets(&levels, 2.0, f64::NEG_INFINITY);
        assert!(on.len() >= 5 && on.len() <= 8, "{on:?}");
        for w in on.windows(2) {
            assert!(w[1] - w[0] >= ONSET_GAP - 1e-9);
        }
        // Silence has none.
        assert!(level_onsets(&[0.01; 100], 1.0, f64::NEG_INFINITY).is_empty());
    }
}
