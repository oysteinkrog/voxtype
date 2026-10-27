//! Letter outlines as paths in the `type` look, each behind a motion
//! feature (`Look::on`):
//!
//! - `glyph-morph` (MO-06): a new word's outlines grow out of a dot under it
//!   and a revised word's out of a line under it, as an accent stroke over
//!   the word; on a cancel they fold back into the line.
//! - `breath-line` (MO-10): before speech the reading line is a hairline
//!   that breathes with the level. At the first word it kinks under the
//!   word, and the word's outlines rise out of the line (60 ms) while the
//!   letters fill (160 ms).
//! - `broken-frame` (MO-53): on a failure the frame snaps at one point and
//!   the ends curl 4 px apart (150 ms), with the failed stage named beside
//!   the break. The words stay still.
//!
//! The words themselves stay the live text. With one of these features on
//! they arrive at 70 % ink without a rise and fill to full, so the reading
//! line keeps its envelope; the outlines are an overlay on top. Reduced
//! motion keeps the hairline and the broken frame as still marks and draws
//! no outline motion.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use vo::scene::canvas::Painter;
use vo::scene::geom::{Affine, Point};
use vo::scene::glyph_morph::{GlyphMorph, MorphFrom, MorphStyle, WordOutline};
use vo::scene::path::{Contour, StrokeStyle};
use vo::scene::snap::{snapped, Snap};
use vo::scene::text::ShapedText;

pub(crate) const MORPH: &str = "glyph-morph";
pub(crate) const BREATH: &str = "breath-line";
pub(crate) const BROKEN: &str = "broken-frame";

/// A new word's outline morph, in seconds.
const MORPH_SECS: f64 = 0.24;
/// The first word's outlines rising out of the breath line, and the fill.
const RISE_SECS: f64 = 0.06;
const FILL_SECS: f64 = 0.16;
/// How long the kink under the first word takes to relax.
const KINK_SECS: f64 = 0.6;
/// The breath line sits this far below the baseline, and the kink lifts it
/// this far, up to the baseline.
const LINE_DROP: f32 = 4.0;
/// The frame snapping apart.
const SNAP_SECS: f64 = 0.15;
/// How much of each broken end lights up.
const END: f32 = 10.0;
/// The ink a word arrives at with an outline feature on.
pub(crate) const ARRIVAL_INK: f32 = 0.7;
/// Points per contour in a morph.
const MORPH_POINTS: usize = 48;

/// Where a word's outlines come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Dot,
    Line,
}

/// Morphs by word id, text, place and source, shared by every popup: a
/// view builds the same morph many times while it plays.
type Key = (u32, String, i32, i32, u32, Source);

fn cache() -> &'static Mutex<HashMap<Key, Arc<GlyphMorph>>> {
    static C: std::sync::OnceLock<Mutex<HashMap<Key, Arc<GlyphMorph>>>> =
        std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// One word of the live text laid out as the live text lays it out: its
/// glyphs in the text box's space and its baseline.
struct Laid {
    id: u32,
    text: String,
    shaped: ShapedText,
    baseline: f32,
    /// Left and right ends of the word's pen box.
    x: (f32, f32),
}

/// One morph to play: the morph, when it starts and how long it runs, and
/// whether it runs back (glyph to source).
#[derive(Clone)]
struct Play {
    morph: Arc<GlyphMorph>,
    start: f64,
    secs: f64,
    /// After the morph lands, the outline stays this long while it fades.
    hold: f64,
    back: bool,
}

impl Play {
    fn t(&self, now: f64) -> f32 {
        let k = ((now - self.start) / self.secs).clamp(0.0, 1.0);
        let e = kit::ease_out(k);
        if self.back {
            1.0 - e
        } else {
            e
        }
    }
}

impl Popup {
    /// Whether an outline feature plays over the live text, so new words
    /// come in at the reading-line ink without a rise.
    pub(crate) fn paths_on(&self) -> bool {
        self.look.on(MORPH) || self.look.on(BREATH)
    }

    fn paths_reduced(&self) -> bool {
        kit::reduced() || self.look.on("reduced-motion")
    }

    /// The shown words laid out as the live text lays them out.
    fn paths_laid(&self) -> Vec<Laid> {
        let l = &self.look;
        let style = l.text_style();
        let mut text = String::new();
        let mut starts = Vec::with_capacity(self.shown.len());
        for (i, w) in self.shown.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            starts.push(text.len() as u32);
            text.push_str(&w.text);
        }
        let shaped = self.measure.layout(&text, &style, Some(l.text_width()));
        let mut out: Vec<Laid> = Vec::new();
        for line in &shaped.lines {
            for g in &shaped.glyphs[line.glyphs.clone()] {
                let Some(i) = starts.partition_point(|&s| s <= g.cluster).checked_sub(1) else {
                    continue;
                };
                let w = &self.shown[i];
                if g.cluster >= starts[i] + w.text.len() as u32 {
                    continue;
                }
                match out.last_mut() {
                    Some(laid) if laid.id == w.id => {
                        laid.shaped.glyphs.push(*g);
                        laid.x.1 = laid.x.1.max(g.pos.x + g.advance);
                    }
                    _ => out.push(Laid {
                        id: w.id,
                        text: w.text.clone(),
                        shaped: ShapedText {
                            glyphs: vec![*g],
                            ..Default::default()
                        },
                        baseline: line.baseline,
                        x: (g.pos.x, g.pos.x + g.advance),
                    }),
                }
            }
        }
        out
    }

    /// The morph of a laid word from `source`, from the shared cache.
    fn paths_morph(&self, w: &Laid, source: Source) -> Arc<GlyphMorph> {
        let size = self.look.text_style().size;
        let key: Key = (
            w.id,
            w.text.clone(),
            (w.x.0 * 4.0) as i32,
            (w.baseline * 4.0) as i32,
            size.to_bits(),
            source,
        );
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = c.get(&key) {
            return m.clone();
        }
        if c.len() > 256 {
            c.clear();
        }
        let word = WordOutline::new(self.measure.fonts(), &w.shaped, size, Point::ZERO);
        let y = w.baseline + LINE_DROP;
        let from = match source {
            Source::Dot => MorphFrom::Blob {
                centre: Point::new(0.5 * (w.x.0 + w.x.1), w.baseline - 0.3 * size),
                radius: 2.0,
            },
            Source::Line => MorphFrom::Segment {
                a: Point::new(w.x.0, y),
                b: Point::new(w.x.1, y),
            },
        };
        let m = Arc::new(GlyphMorph::new(word, from, MORPH_POINTS));
        c.insert(key, m.clone());
        m
    }

    /// The morphs playing now or soon, for the overlay.
    fn paths_plays(&self, laid: &[Laid]) -> Vec<Play> {
        let mut plays = Vec::new();
        if self.paths_reduced() {
            return plays;
        }
        let first = self.shown.first().map(|w| w.id);
        let breath = self.look.on(BREATH);
        let morph = self.look.on(MORPH);
        let cancelled_at = (self.ending() == Some(Ending::Cancelled))
            .then_some(self.ended_at)
            .flatten();
        for w in laid {
            if let (true, Some(at)) = (morph, cancelled_at) {
                plays.push(Play {
                    morph: self.paths_morph(w, Source::Line),
                    start: at,
                    secs: 0.3,
                    hold: 0.3,
                    back: true,
                });
                continue;
            }
            let heard = self.heard.word(w.id);
            let born = heard.map(|h| h.born);
            let revised = heard.and_then(|h| h.changed);
            if breath && Some(w.id) == first && self.cut == 0 {
                if let Some(at) = born {
                    plays.push(Play {
                        morph: self.paths_morph(w, Source::Line),
                        start: at,
                        secs: RISE_SECS,
                        hold: FILL_SECS,
                        back: false,
                    });
                }
                continue;
            }
            if !morph {
                continue;
            }
            match (born, revised) {
                (_, Some(at)) if born.is_none_or(|b| at > b) => plays.push(Play {
                    morph: self.paths_morph(w, Source::Line),
                    start: at,
                    secs: MORPH_SECS,
                    hold: 0.0,
                    back: false,
                }),
                (Some(at), _) => plays.push(Play {
                    morph: self.paths_morph(w, Source::Dot),
                    start: at,
                    secs: MORPH_SECS,
                    hold: 0.0,
                    back: false,
                }),
                _ => {}
            }
        }
        plays
    }

    /// The overlays for the text column: the breath line under the reading
    /// line and the outline morphs over the words, both in the text box's
    /// space.
    pub(crate) fn paths_overlays(&self) -> Vec<Element<Msg>> {
        if !self.paths_on() {
            return Vec::new();
        }
        let l = &self.look;
        let laid = self.paths_laid();
        let plays = self.paths_plays(&laid);
        let w = l.text_width();
        let h = self.lines as f32 * l.text_line;
        let mut out = Vec::new();
        if self.look.on(BREATH) {
            out.push(kit::at(self.breath_line(&laid, w), 0.0, 0.0).size(w, h));
        }
        let now = self.extra.now;
        let until = plays
            .iter()
            .map(|p| p.start + p.secs + p.hold)
            .fold(0.0, f64::max);
        if until > now - 0.05 {
            let style = MorphStyle::new(StrokeStyle::solid(1.2, l.accent), l.foreground.fade(0.0))
                .fill_from(1.0)
                .outline_out(0.35);
            let draw = move |t: f64, r: Rect, p: &mut Painter| {
                p.layer(Affine::translate(r.x, r.y), 1.0, |p| {
                    for play in &plays {
                        if t < play.start {
                            continue;
                        }
                        let k = play.t(t);
                        let end = play.start + play.secs;
                        if t >= end + play.hold {
                            continue;
                        }
                        // A held outline (the fold back, the first word's rise)
                        // stays whole to its end, then fades over the hold.
                        let style = if play.hold > 0.0 {
                            let left = (1.0 - (t - end) / play.hold).clamp(0.0, 1.0) as f32;
                            MorphStyle {
                                stroke: StrokeStyle {
                                    c0: style.stroke.c0.fade(left),
                                    c1: style.stroke.c1.fade(left),
                                    ..style.stroke
                                },
                                ..style.outline_out(0.0)
                            }
                        } else if k >= 1.0 {
                            continue;
                        } else {
                            style
                        };
                        play.morph.draw(p, k.min(0.999), &style);
                    }
                });
            };
            out.push(kit::at(Element::canvas("glyph-paths", until, draw), 0.0, 0.0).size(w, h));
        }
        out
    }

    /// The breath line under the reading line: a hairline breathing with
    /// the level, kinked under the first word as it arrives.
    fn breath_line(&self, laid: &[Laid], w: f32) -> Element<Msg> {
        let l = &self.look;
        let reduced = self.paths_reduced();
        let style = l.text_style();
        // The reading line's baseline: the newest word's, or the first
        // line's before any word.
        let baseline = laid.last().map(|x| x.baseline).unwrap_or_else(|| {
            self.measure
                .layout("H", &style, None)
                .lines
                .first()
                .map_or(style.size, |x| x.baseline)
        });
        let y = baseline + LINE_DROP;
        let first = laid
            .first()
            .filter(|_| self.cut == 0)
            .and_then(|x| Some((x.x, self.heard.word(x.id)?.born)));
        let levels = kit::Levels {
            levels: self.wave.clone(),
            newest_at: self.newest_at,
        };
        let color = kit::mix(l.border, l.foreground, 0.35);
        let listening = !self.ended();
        // Breathing stops half a second after the newest level; the kink
        // relaxes on its own clock.
        let until = if reduced {
            0.0
        } else {
            let breathing = if listening { self.newest_at + 0.5 } else { 0.0 };
            breathing.max(first.map_or(0.0, |f| f.1 + KINK_SECS))
        };
        Element::canvas("breath-line", until, move |t, r, p| {
            p.layer(Affine::translate(r.x, r.y), 1.0, |p| {
                let level = if reduced || !listening {
                    0.0
                } else {
                    levels.at(t).clamp(0.0, 1.0)
                };
                let width = 1.0 + level;
                let pen = StrokeStyle::solid(width, color);
                let kink = match first {
                    Some(((x0, x1), at)) if !reduced && t >= at => {
                        let u = (t - at) / KINK_SECS;
                        // Up in 60 ms, then relaxing to flat.
                        let rise = ((t - at) / RISE_SECS).clamp(0.0, 1.0) as f32;
                        let relax = 1.0 - kit::ease_in_out(u);
                        (u < 1.0).then_some((x0, x1, rise * relax))
                    }
                    _ => None,
                };
                match kink {
                    None => p.stroke(
                        &Contour::polyline(&[Point::new(0.0, y), Point::new(w, y)], false),
                        &pen,
                    ),
                    Some((x0, x1, k)) => {
                        let lift = LINE_DROP * k;
                        let shoulder = 8.0f32;
                        let (a, b) = ((x0 - shoulder).max(0.0), (x1 + shoulder).min(w));
                        p.stroke(
                            &Contour::polyline(&[Point::new(0.0, y), Point::new(a, y)], false),
                            &pen,
                        );
                        // The kink: smooth shoulders up to the baseline under the word.
                        let mut pts = Vec::new();
                        let n = 24;
                        for i in 0..=n {
                            let x = a + (b - a) * i as f32 / n as f32;
                            let d = (x0 - x).max(x - x1).max(0.0) / shoulder;
                            let s = 1.0 - d.min(1.0);
                            let s = s * s * (3.0 - 2.0 * s);
                            pts.push(Point::new(x, y - lift * s));
                        }
                        p.stroke(&Contour::polyline(&pts, false), &pen);
                        p.stroke(
                            &Contour::polyline(&[Point::new(b, y), Point::new(w, y)], false),
                            &pen,
                        );
                    }
                }
            })
        })
    }

    /// The type look's frame snapped open on a failure, or `None` when the
    /// feature is off or nothing failed.
    pub(crate) fn paths_broken_frame(
        &self,
        border: &Contour,
        w: f32,
        h: f32,
        width: f32,
    ) -> Option<Element<Msg>> {
        if !self.look.on(BROKEN) || self.ending() != Some(Ending::Failed) {
            return None;
        }
        let l = &self.look;
        let at = self.ended_at.unwrap_or(self.extra.now);
        let reduced = self.paths_reduced();
        // Break on the top edge, under the gap between the title and the clock.
        let len = border.length();
        let top = (0..512)
            .map(|i| len * i as f32 / 512.0)
            .filter_map(|s| border.at(s).map(|(p, _)| (s, p)))
            .filter(|(_, p)| p.y < 2.0)
            .min_by(|a, b| {
                (a.1.x - 0.45 * w)
                    .abs()
                    .total_cmp(&(b.1.x - 0.45 * w).abs())
            })
            .map_or(0.0, |x| x.0);
        // The ends curl into the panel: above the top edge is the surface's end.
        let snap = Snap {
            curl: -4.0,
            ..Snap::new(top)
        };
        let border = border.clone();
        let (line, accent) = (l.border, l.accent);
        // The stage that failed, from how far the dictation came.
        let stage = if self.extra.finish_at.is_some() || self.was_finishing {
            "final pass failed"
        } else {
            "recording failed"
        };
        let label_style = l.mono(l.caption_small_px);
        let shaped = self.measure.layout(stage, &label_style, None);
        let break_x = border.at(top).map_or(0.45 * w, |x| x.0.x);
        let lx = (break_x + 10.0).min(w - shaped.size.w - 4.0);
        let glyphs: Vec<_> = shaped
            .glyphs
            .iter()
            .map(|g| (g.font, g.glyph, Point::new(lx + g.pos.x, 3.0 + g.pos.y)))
            .collect();
        let size = label_style.size;
        let until = if reduced { 0.0 } else { at + SNAP_SECS };
        let canvas = Element::canvas("frame-broken", until, move |t, r, p| {
            p.layer(Affine::translate(r.x, r.y), 1.0, |p| {
                let k = if reduced {
                    1.0
                } else {
                    kit::ease_out((t - at) / SNAP_SECS)
                };
                for c in snapped(&border, &snap, k) {
                    p.stroke(&c, &StrokeStyle::solid(width, line));
                    // The two ends light up in the accent as they part.
                    let len = c.length();
                    let tip = StrokeStyle::solid(width.max(1.2), accent.fade(k));
                    p.stroke(&c.trim(0.0, END), &tip);
                    p.stroke(&c.trim(len - END, len), &tip);
                }
                for &(font, glyph, pos) in &glyphs {
                    p.glyph(vo::scene::display::GlyphInstance {
                        pos,
                        size,
                        font,
                        glyph,
                        word: vo::scene::display::NO_WORD,
                        color: accent.fade(k),
                    });
                }
            })
        });
        Some(kit::at(canvas, 0.0, 0.0).size(w, h))
    }
}

impl Popup {
    /// How a new word enters the type look: `normal`, or with an outline
    /// feature on, at the arrival ink with no rise.
    pub(crate) fn paths_word_enter(&self, normal: Transition) -> Transition {
        if self.paths_on() {
            Transition::fade().opacity(ARRIVAL_INK)
        } else {
            normal
        }
    }
}
