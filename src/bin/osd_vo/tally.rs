//! Word arrivals, revisions and the speaking pace, for the looks that give
//! each new word its own choreography: the count that takes a cast (MO-65),
//! the leaning pace digits (MO-66), dot sortation (MO-60), the beam that
//! writes words (MO-61) and the island's word bulge (MO-74).
//!
//! The state follows the words on screen by id, so a look can ask when a
//! word came, which of its letters a revision changed, and where the word
//! sits in the transcript.
//!
//! Each concept is opt-in per look through its motion feature
//! (`--motion NAME`, or a station exhibit): `count-cast`, `lean-pace`,
//! `dot-sort`, `beam-write` and `word-bulge`. Under reduced motion each one
//! keeps its state as a still mark.

use super::kit::{ease_in_out, ease_out, mix, reduced};
use super::*;

use std::collections::VecDeque;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};

/// The cast: the block shows over the word, slides and contracts into the
/// notch, the counter rolls as it lands, and the notch settles.
pub(crate) const CAST_SHOW: f64 = 0.13;
pub(crate) const CAST_LAND: f64 = 0.26;
pub(crate) const CAST_SECS: f64 = 0.52;
/// Words that come in one update cast one after the other, this far apart.
const CAST_STAGGER: f64 = 0.06;
/// The pace digits lean at most this far, in radians (8 degrees).
pub(crate) const LEAN_MAX: f32 = 8.0 * std::f32::consts::PI / 180.0;
/// Recent pace, over this many seconds of word arrivals.
const PACE_WINDOW: f64 = 3.0;
/// The average pace, over at most this many seconds.
const WPM_WINDOW: f64 = 12.0;
/// Below this level for this long, the speaker has paused.
const PAUSE_LEVEL: f32 = 0.06;
const PAUSE_SECS: f64 = 0.35;
/// Arrivals and revisions kept.
const KEEP: usize = 16;

/// A word whose text changed: which letters, and when.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Revision {
    pub id: u32,
    /// The first changed letter, counted in chars.
    pub from: usize,
    pub at: f64,
}

/// What the tally keeps between messages.
#[derive(Default)]
pub(crate) struct State {
    /// The words on screen last time, by id.
    seen: Vec<(u32, String)>,
    /// New words and when each came, oldest first. Words that came in one
    /// update are spread by `CAST_STAGGER`.
    pub arrived: VecDeque<(u32, f64)>,
    /// The latest revision of each recently revised word.
    pub revised: VecDeque<Revision>,
    /// When each counted word came, for the pace.
    times: VecDeque<f64>,
    /// The word count and when it changed, oldest first.
    counts: VecDeque<(usize, f64)>,
}

/// Where a shown word sits in the transcript: its box and the left edge
/// and width of each letter, in the transcript's coordinates.
#[derive(Clone, Debug)]
pub(crate) struct WordBox {
    pub rect: Rect,
    pub baseline: f32,
    pub letters: Vec<(f32, f32)>,
}

/// The common prefix of two words, in chars.
pub(crate) fn changed_from(old: &str, new: &str) -> usize {
    old.chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

impl State {
    /// Brings the tally up to date with the words on screen at `t`.
    /// `counting` is false once the final text came, so its new words are
    /// not taken as speech.
    pub(crate) fn sync(&mut self, shown: &[Word], count: usize, counting: bool, t: f64) {
        let mut fresh = 0usize;
        for w in shown {
            match self.seen.iter().find(|(id, _)| *id == w.id) {
                None if counting && !self.arrived.iter().any(|(id, _)| *id == w.id) => {
                    let at = t + fresh as f64 * CAST_STAGGER;
                    self.arrived.push_back((w.id, at));
                    self.times.push_back(at);
                    fresh += 1;
                }
                Some((_, old)) if old != &w.text && counting => {
                    let from = changed_from(old, &w.text);
                    self.revised.retain(|r| r.id != w.id);
                    self.revised.push_back(Revision {
                        id: w.id,
                        from,
                        at: t,
                    });
                }
                _ => {}
            }
        }
        self.seen = shown.iter().map(|w| (w.id, w.text.clone())).collect();
        while self.arrived.len() > KEEP {
            self.arrived.pop_front();
        }
        while self.revised.len() > KEEP {
            self.revised.pop_front();
        }
        while self.times.front().is_some_and(|&a| a < t - WPM_WINDOW) {
            self.times.pop_front();
        }
        if self.counts.back().map(|c| c.0) != Some(count) {
            // The count lands with the last cast of this update.
            let land = t + fresh.saturating_sub(1) as f64 * CAST_STAGGER + CAST_LAND;
            self.counts
                .push_back((count, if fresh > 0 { land } else { t }));
            while self.counts.len() > KEEP {
                self.counts.pop_front();
            }
        }
    }

    /// The latest revision of word `id`.
    pub(crate) fn revision(&self, id: u32) -> Option<&Revision> {
        self.revised.iter().find(|r| r.id == id)
    }

    /// The word count to show at `t`: a counted word shows once its cast
    /// lands, or at once when `held` is false.
    pub(crate) fn count_at(&self, t: f64, held: bool) -> usize {
        if !held {
            return self.counts.back().map_or(0, |c| c.0);
        }
        self.counts
            .iter()
            .rev()
            .find(|c| c.1 <= t)
            .or(self.counts.front())
            .map_or(0, |c| c.0)
    }

    /// Words per minute: the words that came over the last few seconds
    /// against the time they took. 0 before two words.
    pub(crate) fn wpm(&self, t: f64) -> u32 {
        let n = self.times.len();
        if n < 2 {
            return 0;
        }
        let first = self.times[0];
        let span = (t - first).clamp(2.0, WPM_WINDOW);
        (n as f64 * 60.0 / span).round().min(999.0) as u32
    }

    /// Words per minute over the last `PACE_WINDOW` seconds.
    pub(crate) fn recent_wpm(&self, t: f64) -> f32 {
        let n = self
            .times
            .iter()
            .filter(|&&a| a > t - PACE_WINDOW && a <= t)
            .count();
        (n as f64 * 60.0 / PACE_WINDOW) as f32
    }
}

/// How far the pace digits lean: forward with the recent pace, upright in a
/// pause or once the recording stops, and never under reduced motion.
pub(crate) fn lean(recent_wpm: f32, speaking: bool) -> f32 {
    if !speaking {
        return 0.0;
    }
    LEAN_MAX * ((recent_wpm - 60.0) / 120.0).clamp(0.0, 1.0)
}

/// The speaker is talking: the level stayed up within the last
/// `PAUSE_SECS`.
pub(crate) fn speaking(levels: &kit::Levels, t: f64) -> bool {
    (0..8).any(|i| levels.at(t - PAUSE_SECS * i as f64 / 7.0) > PAUSE_LEVEL)
}

/// `text` with each character its own label, leaning `lean` radians
/// forward (clockwise) on a spring, so the digits tip over as the pace
/// rises and stand up in a pause. Every digit cell is as wide as `cell`.
pub(crate) fn lean_digits<M>(
    key: &str,
    text: &str,
    style: TextStyle,
    color: Color,
    cell: f32,
    lean: f32,
) -> Element<M> {
    let n = text.chars().count();
    Element::row(key.to_string()).children(text.chars().enumerate().map(|(i, c)| {
        // Keyed by place from the right, so a digit keeps its lean as the
        // number grows.
        Element::label(format!("{key}-{}", n - 1 - i), c.to_string())
            .text_style(style)
            .color(color)
            .width(cell)
            .rotate(lean)
            .motion(Prop::Rotation, Spring::critical_to_98(0.3))
    }))
}

/// One word's cast, in the canvas's coordinates: the word's box, the notch
/// and when the word came.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cast {
    pub from: Rect,
    pub at: f64,
}

/// The casts of one look and the notch they land in, drawn on a canvas
/// under the transcript.
#[derive(Clone)]
pub(crate) struct Casts {
    pub casts: Vec<Cast>,
    /// The notch's box.
    pub notch: Rect,
    pub accent: Color,
    /// The look's own material: a dotted block for the dot looks.
    pub dots: bool,
    /// Only the settled notch, for reduced motion and the end of a
    /// dictation.
    pub still: bool,
}

impl Casts {
    pub(crate) fn until(&self) -> f64 {
        self.casts
            .iter()
            .map(|c| c.at + CAST_SECS)
            .fold(0.0, f64::max)
    }

    /// The block of cast `c` at `t`, with its opacity, or `None` before it
    /// shows and after it landed.
    pub(crate) fn block(&self, c: &Cast, t: f64) -> Option<(Rect, f32)> {
        let s = t - c.at;
        if !(0.0..CAST_LAND).contains(&s) {
            return None;
        }
        let n = self.notch;
        if s < CAST_SHOW {
            // The block rises from the baseline to the word's height.
            let k = ease_out(s / CAST_SHOW);
            let h = c.from.h * (0.3 + 0.7 * k);
            return Some((
                Rect::new(c.from.x, c.from.y + c.from.h - h, c.from.w, h),
                0.18 + 0.12 * k,
            ));
        }
        // It slides into the notch and contracts to the notch's size.
        let k = ease_in_out((s - CAST_SHOW) / (CAST_LAND - CAST_SHOW));
        let lerp = |a: f32, b: f32| a + (b - a) * k;
        Some((
            Rect::new(
                lerp(c.from.x, n.x),
                lerp(c.from.y, n.y),
                lerp(c.from.w, n.w),
                lerp(c.from.h, n.h),
            ),
            0.3 + 0.7 * k,
        ))
    }

    /// How bright the notch is at `t`: a flash when a cast lands that
    /// settles back.
    pub(crate) fn notch_glow(&self, t: f64) -> f32 {
        self.casts
            .iter()
            .map(|c| {
                let s = t - c.at - CAST_LAND;
                if (0.0..CAST_SECS - CAST_LAND).contains(&s) {
                    1.0 - ease_out(s / (CAST_SECS - CAST_LAND))
                } else {
                    0.0
                }
            })
            .fold(0.0, f32::max)
    }

    pub(crate) fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let o = Point::new(r.x, r.y);
        let shift = |b: Rect| Rect::new(b.x + o.x, b.y + o.y, b.w, b.h);
        if !self.still {
            for c in &self.casts {
                let Some((b, a)) = self.block(c, t) else {
                    continue;
                };
                // A block of dots reads heavier than a solid one.
                let a = if self.dots { a * 0.7 } else { a };
                self.paint_block(shift(b), self.accent.fade(a), p);
            }
        }
        let glow = if self.still { 0.0 } else { self.notch_glow(t) };
        let n = shift(self.notch);
        let c = mix(self.accent.fade(0.85), Color::WHITE, 0.5 * glow);
        if glow > 0.0 {
            p.shape(ShapeInstance::rrect(
                RRect::new(n, n.w / 2.0),
                Paint::glow(self.accent.fade(0.35 * glow), 4.0, 0.5),
            ));
        }
        self.paint_block(n, c, p);
    }

    fn paint_block(&self, b: Rect, c: Color, p: &mut Painter) {
        if !self.dots {
            p.shape(ShapeInstance::rrect(
                RRect::new(b, (b.w.min(b.h) * 0.25).min(3.0)),
                Paint::solid(c),
            ));
            return;
        }
        // A block of dots on a 4 px pitch, at least one dot across.
        let pitch = 4.0;
        let cols = ((b.w / pitch).floor() as usize).max(1);
        let rows = ((b.h / pitch).floor() as usize).max(1);
        let dx = b.w / cols as f32;
        let dy = b.h / rows as f32;
        let rad = (dx.min(dy) * 0.38).clamp(0.6, 1.5);
        for i in 0..cols {
            for j in 0..rows {
                p.shape(ShapeInstance::circle(
                    Point::new(b.x + (i as f32 + 0.5) * dx, b.y + (j as f32 + 0.5) * dy),
                    rad,
                    Paint::solid(c),
                ));
            }
        }
    }
}

impl Popup {
    /// Motion feature `name` is on for this look.
    pub(crate) fn motion(&self, name: &str) -> bool {
        self.look.on(name)
    }

    /// Reduced motion, from the environment or this look's features.
    pub(crate) fn still(&self) -> bool {
        reduced() || self.look.on("reduced-motion")
    }

    /// Brings the tally up to date after a message.
    pub(crate) fn sync_tally(&mut self, t: f64) {
        let counting = self.extra.final_at.is_none();
        self.tally.sync(&self.shown, self.word_count, counting, t);
    }

    /// The box and letters of shown word `i`, laid out in `style` and
    /// wrapped at `width` as the transcript is.
    pub(crate) fn tally_word_box(
        &self,
        i: usize,
        style: &TextStyle,
        width: f32,
    ) -> Option<WordBox> {
        let mut text = String::new();
        let mut start = 0u32;
        for (j, w) in self.shown.iter().enumerate() {
            if !text.is_empty() {
                text.push(' ');
            }
            if j == i {
                start = text.len() as u32;
            }
            text.push_str(&w.text);
        }
        let w = self.shown.get(i)?;
        let end = start + w.text.len() as u32;
        let shaped = self.measure.layout(&text, style, Some(width));
        let line = shaped.lines.iter().position(|l| {
            shaped.glyphs[l.glyphs.clone()]
                .iter()
                .any(|g| g.cluster >= start && g.cluster < end)
        })?;
        let l = &shaped.lines[line];
        let glyphs: Vec<_> = shaped.glyphs[l.glyphs.clone()]
            .iter()
            .filter(|g| g.cluster >= start && g.cluster < end)
            .collect();
        let x0 = glyphs.iter().map(|g| g.pos.x).fold(f32::MAX, f32::min);
        let x1 = glyphs
            .iter()
            .map(|g| g.pos.x + g.advance)
            .fold(f32::MIN, f32::max);
        let letters = glyphs.iter().map(|g| (g.pos.x, g.advance)).collect();
        Some(WordBox {
            rect: Rect::new(
                x0,
                line as f32 * style.line_height,
                x1 - x0,
                style.line_height,
            ),
            baseline: l.baseline,
            letters,
        })
    }

    /// The recent arrivals that are still on screen, with their boxes in the
    /// transcript, newest last.
    pub(crate) fn tally_arrivals(
        &self,
        style: &TextStyle,
        width: f32,
        within: f64,
    ) -> Vec<(WordBox, f64)> {
        let now = self.extra.now;
        self.tally
            .arrived
            .iter()
            .filter(|(_, at)| now - at < within)
            .filter_map(|&(id, at)| {
                let i = self.shown.iter().position(|w| w.id == id)?;
                Some((self.tally_word_box(i, style, width)?, at))
            })
            .collect()
    }

    /// The casts of the recent arrivals, offset by `origin` (the
    /// transcript's top left in the canvas), landing in `notch`.
    pub(crate) fn tally_casts(
        &self,
        origin: Point,
        notch: Rect,
        accent: Color,
        dots: bool,
    ) -> Casts {
        let style = self.look.text_style();
        let width = self.look.text_width();
        let still = self.still() || self.extra.final_at.is_some();
        let casts = if still {
            Vec::new()
        } else {
            self.tally_arrivals(&style, width, CAST_SECS)
                .into_iter()
                .map(|(b, at)| {
                    // The block hugs the word's ink, not the whole line.
                    let h = style.size * 0.9;
                    let y = b.baseline - style.size * 0.75;
                    Cast {
                        from: Rect::new(
                            origin.x + b.rect.x - 2.0,
                            origin.y + y.min(b.rect.y + b.rect.h - h),
                            b.rect.w + 4.0,
                            h,
                        ),
                        at,
                    }
                })
                .collect()
        };
        Casts {
            casts,
            notch,
            accent,
            dots,
            still,
        }
    }

    /// The word count to show now: with `count-cast`, held until its cast
    /// lands.
    pub(crate) fn tally_count(&self) -> String {
        let held = self.motion("count-cast") && !self.still();
        self.tally.count_at(self.extra.now, held).to_string()
    }

    /// The pace in words per minute and the lean of its digits now.
    pub(crate) fn tally_pace(&self) -> (u32, f32) {
        let t = self
            .extra
            .finish_at
            .map_or(self.extra.now, |f| f.min(self.extra.now));
        let live = self.extra.finish_at.is_none() && !self.ended() && !self.status().finishing;
        let talking = live && !self.still() && speaking(&self.kit_raw_levels(), self.extra.now);
        (self.tally.wpm(t), lean(self.tally.recent_wpm(t), talking))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(ws: &[(u32, &str)]) -> Vec<Word> {
        ws.iter()
            .map(|&(id, text)| Word {
                id,
                text: text.to_string(),
            })
            .collect()
    }

    fn arrived_at(s: &State, id: u32) -> Option<f64> {
        s.arrived.iter().find(|(i, _)| *i == id).map(|a| a.1)
    }

    #[test]
    fn new_words_arrive_staggered_and_revisions_name_the_changed_letters() {
        let mut s = State::default();
        s.sync(&words(&[(1, "the")]), 1, true, 1.0);
        s.sync(
            &words(&[(1, "the"), (2, "quick"), (3, "fox")]),
            3,
            true,
            2.0,
        );
        assert_eq!(arrived_at(&s, 1), Some(1.0));
        assert_eq!(arrived_at(&s, 2), Some(2.0));
        assert_eq!(arrived_at(&s, 3), Some(2.0 + CAST_STAGGER));
        s.sync(
            &words(&[(1, "the"), (2, "quack"), (3, "fox")]),
            3,
            true,
            3.0,
        );
        assert_eq!(s.revision(2).map(|r| r.from), Some(2));
        assert_eq!(s.revision(1), None);
    }

    #[test]
    fn the_count_rolls_when_the_cast_lands() {
        let mut s = State::default();
        s.sync(&words(&[(1, "a")]), 1, true, 1.0);
        s.sync(&words(&[(1, "a"), (2, "b")]), 2, true, 2.0);
        assert_eq!(s.count_at(2.0, true), 1);
        assert_eq!(s.count_at(2.0 + CAST_LAND, true), 2);
        assert_eq!(s.count_at(2.0, false), 2);
    }

    #[test]
    fn the_final_text_is_not_counted_as_speech() {
        let mut s = State::default();
        s.sync(&words(&[(1, "a")]), 1, true, 1.0);
        s.sync(&words(&[(7, "A.")]), 1, false, 2.0);
        assert_eq!(arrived_at(&s, 7), None);
        assert!(s.revised.is_empty());
    }

    #[test]
    fn pace_counts_words_over_their_time_and_lean_stays_in_bounds() {
        let mut s = State::default();
        let mut shown = Vec::new();
        for i in 0..10u32 {
            shown.push(Word {
                id: i,
                text: "w".into(),
            });
            s.sync(&shown, shown.len(), true, i as f64 * 0.4);
        }
        // Ten words in 3.6 s, read at 4 s: 150 a minute.
        assert_eq!(s.wpm(4.0), 150);
        assert!(s.recent_wpm(3.6) > 100.0);
        assert!(lean(400.0, true) <= LEAN_MAX + 1e-6);
        assert_eq!(lean(400.0, false), 0.0);
        assert_eq!(lean(30.0, true), 0.0);
    }

    #[test]
    fn a_cast_contracts_into_the_notch() {
        let casts = Casts {
            casts: vec![Cast {
                from: Rect::new(10.0, 50.0, 60.0, 20.0),
                at: 0.0,
            }],
            notch: Rect::new(300.0, 5.0, 3.0, 12.0),
            accent: Color::WHITE,
            dots: false,
            still: false,
        };
        let c = casts.casts[0];
        let (start, _) = casts.block(&c, 0.0).unwrap();
        assert!((start.x - 10.0).abs() < 1e-3 && (start.w - 60.0).abs() < 1e-3);
        let (end, _) = casts.block(&c, CAST_LAND - 1e-4).unwrap();
        assert!((end.x - 300.0).abs() < 0.5 && (end.w - 3.0).abs() < 0.5);
        assert!(casts.block(&c, CAST_LAND + 0.01).is_none());
        assert!(casts.notch_glow(CAST_LAND + 0.01) > 0.8);
        assert_eq!(casts.notch_glow(CAST_SECS + 0.01), 0.0);
    }
}
