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
//! line keeps its envelope; the outlines are an overlay on top. With
//! `stroke-write` (SW-02) the pen does the filling: a word arrives at once,
//! its letters at 70 % ink, and each letter goes to full ink as the pen
//! writes it ([`Popup::pen_fills`], applied by `Popup::typo_words`).
//! Reduced motion keeps the hairline and the broken frame as still marks
//! and draws no outline motion and no pen, and words arrive at full ink.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use vo::scene::canvas::Painter;
use vo::scene::centreline;
use vo::scene::geom::{Affine, Point};
use vo::scene::glyph_morph::{GlyphMorph, MorphFrom, MorphStyle, WordOutline};
use vo::scene::path::{Contour, StrokeStyle};
use vo::scene::snap::{snapped, Snap};
use vo::scene::stroke_write::{PenParams, PenSchedule, PenSlot, StrokeSource, WetInk, Writing};
use vo::scene::text::ShapedText;

pub(crate) const MORPH: &str = "glyph-morph";
pub(crate) const BREATH: &str = "breath-line";
pub(crate) const BROKEN: &str = "broken-frame";
pub(crate) const WRITE: &str = "stroke-write";

/// Letters whose pen strokes are analysed on the worker when stroke
/// writing first shows, so a live word rarely waits for its centre lines.
const PEN_PREFETCH: &str =
    "abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 æøåÆØÅéüöä.,?!'-";
/// Pen speed in text sizes per second. At 22.7 px text, "the" is 135 px
/// of pen path (about 170 ms) and "dictation" 334 px (the 260 ms cap).
const PEN_SPEED: f32 = 35.0;

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
/// The ink a letter waits at until the pen writes it (SW-02): the reading
/// line's 70 % floor, a hair over so float rounding never dips under it.
pub(crate) const PEN_FLOOR: f32 = 0.705;
/// Points per contour in a morph.
const MORPH_POINTS: usize = 48;

/// Where a word's outlines come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Dot,
    Line,
}

/// Morphs by word id, text, place, font, size and source, shared by every
/// popup: a view builds the same morph many times while it plays.
type Key = (u32, String, i32, i32, u32, u32, Source);

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

/// How a look draws its broken frame (MO-53): the stroke's width, colour
/// and glow (0 for none), the accent its parted ends light up in, and the
/// stage label's style and top, in the frame's box.
pub(crate) struct BrokenPen {
    pub(crate) width: f32,
    pub(crate) glow: f32,
    pub(crate) line: Color,
    pub(crate) accent: Color,
    pub(crate) label: TextStyle,
    pub(crate) label_y: f32,
}

/// The transcript the glyph paths follow (RD-10): the calling look's text
/// style, the width its live text wraps at and its line pitch. The
/// outlines and pen strokes are built from this style, so they sit on the
/// look's own letters. `ink` and `nib` colour the pen's wet ink and its
/// light: the desktop's text and accent unless the look sets its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PathsText {
    pub(crate) style: TextStyle,
    pub(crate) width: f32,
    pub(crate) line: f32,
    pub(crate) ink: Color,
    pub(crate) nib: Color,
}

impl PathsText {
    /// The look's transcript as `Look` describes it.
    pub(crate) fn of(look: &Look) -> PathsText {
        PathsText {
            style: look.text_style(),
            width: look.text_width(),
            line: look.text_line,
            ink: look.foreground,
            nib: look.accent,
        }
    }
}

impl Popup {
    /// Whether an outline feature plays over the live text, so new words
    /// come in at the reading-line ink without a rise.
    pub(crate) fn paths_on(&self) -> bool {
        self.look.on(MORPH) || self.look.on(BREATH) || self.look.on(WRITE)
    }

    fn paths_reduced(&self) -> bool {
        kit::reduced() || self.look.on("reduced-motion")
    }

    /// The shown words laid out as the live text in `text` lays them out.
    fn paths_laid(&self, text: &PathsText) -> Vec<Laid> {
        let style = text.style;
        let mut joined = String::new();
        let mut starts = Vec::with_capacity(self.shown.len());
        for (i, w) in self.shown.iter().enumerate() {
            if i > 0 {
                joined.push(' ');
            }
            starts.push(joined.len() as u32);
            joined.push_str(&w.text);
        }
        let shaped = self.measure.layout(&joined, &style, Some(text.width));
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
    fn paths_morph(&self, w: &Laid, style: &TextStyle, source: Source) -> Arc<GlyphMorph> {
        let size = style.size;
        let key: Key = (
            w.id,
            w.text.clone(),
            (w.x.0 * 4.0) as i32,
            (w.baseline * 4.0) as i32,
            style.font,
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
    fn paths_plays(&self, laid: &[Laid], style: &TextStyle) -> Vec<Play> {
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
                    morph: self.paths_morph(w, style, Source::Line),
                    start: at,
                    secs: 0.3,
                    hold: 0.3,
                    back: true,
                });
                continue;
            }
            let (born, revised) = self.paths_times(w);
            let born = Some(born);
            if breath && Some(w.id) == first && self.cut == 0 {
                if let Some(at) = born {
                    plays.push(Play {
                        morph: self.paths_morph(w, style, Source::Line),
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
                    morph: self.paths_morph(w, style, Source::Line),
                    start: at,
                    secs: MORPH_SECS,
                    hold: 0.0,
                    back: false,
                }),
                (Some(at), _) => plays.push(Play {
                    morph: self.paths_morph(w, style, Source::Dot),
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

    /// The overlays for the text column of `text`: the breath line under
    /// the reading line and the outline morphs over the words, both in the
    /// text box's space.
    pub(crate) fn paths_overlays(&self, text: &PathsText) -> Vec<Element<Msg>> {
        if !self.paths_on() {
            return Vec::new();
        }
        let l = &self.look;
        let laid = self.paths_laid(text);
        let plays = self.paths_plays(&laid, &text.style);
        let w = text.width;
        let h = self.lines as f32 * text.line;
        let mut out = Vec::new();
        if self.look.on(BREATH) {
            out.push(kit::at(self.breath_line(&laid, &text.style, w), 0.0, 0.0).size(w, h));
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
        if self.look.on(WRITE) {
            out.extend(self.writing_overlay(&laid, text, w, h));
        }
        out
    }

    /// The breath line under the reading line: a hairline breathing with
    /// the level, kinked under the first word as it arrives.
    fn breath_line(&self, laid: &[Laid], style: &TextStyle, w: f32) -> Element<Msg> {
        let l = &self.look;
        let reduced = self.paths_reduced();
        // The reading line's baseline: the newest word's, or the first
        // line's before any word.
        let baseline = laid.last().map(|x| x.baseline).unwrap_or_else(|| {
            self.measure
                .layout("H", style, None)
                .lines
                .first()
                .map_or(style.size, |x| x.baseline)
        });
        let y = baseline + LINE_DROP;
        let first = laid
            .first()
            .filter(|_| self.cut == 0)
            .map(|x| (x.x, self.paths_times(x).0));
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
        if !self.paths_broken() {
            return None;
        }
        let l = &self.look;
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
        let pen = BrokenPen {
            width,
            glow: 0.0,
            line: l.border,
            accent: l.accent,
            label: l.mono(l.caption_small_px),
            label_y: 3.0,
        };
        self.paths_broken_frame_at(border, top, w, h, pen)
    }

    /// Whether the broken frame shows: the feature is on and the dictation
    /// failed. A look that draws its own frame leaves out the part that
    /// `paths_broken_frame_at` draws snapped.
    pub(crate) fn paths_broken(&self) -> bool {
        self.look.on(BROKEN) && self.ending() == Some(Ending::Failed)
    }

    /// Any look's frame snapped open on a failure: `border` breaks `top`
    /// along its length, drawn with `pen`, in a `w` by `h` box. `None` when
    /// the feature is off or nothing failed.
    pub(crate) fn paths_broken_frame_at(
        &self,
        border: &Contour,
        top: f32,
        w: f32,
        h: f32,
        pen: BrokenPen,
    ) -> Option<Element<Msg>> {
        if !self.paths_broken() {
            return None;
        }
        let at = self.ended_at.unwrap_or(self.extra.now);
        let reduced = self.paths_reduced();
        // The ends curl into the panel: above the top edge is the surface's end.
        let snap = Snap {
            curl: -4.0,
            ..Snap::new(top)
        };
        // A look may break one open edge of its frame rather than a closed
        // border.
        let open = match (border.segs.first(), border.segs.last()) {
            (Some(a), Some(b)) => (a.p0.x - b.p2.x).abs() + (a.p0.y - b.p2.y).abs() > 0.5,
            _ => false,
        };
        let border = border.clone();
        let BrokenPen {
            width,
            glow,
            line,
            accent,
            label: label_style,
            label_y,
        } = pen;
        // The stage that failed, from how far the dictation came.
        let stage = if self.extra.finish_at.is_some() || self.was_finishing {
            "final pass failed"
        } else {
            "recording failed"
        };
        let shaped = self.measure.layout(stage, &label_style, None);
        let break_x = border.at(top).map_or(0.45 * w, |x| x.0.x);
        let lx = (break_x + 10.0).min(w - shaped.size.w - 4.0);
        let glyphs: Vec<_> = shaped
            .glyphs
            .iter()
            .map(|g| (g.font, g.glyph, Point::new(lx + g.pos.x, label_y + g.pos.y)))
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
                let tip = StrokeStyle::solid(width.max(1.2), accent.fade(k));
                if open {
                    // An open edge parts into two pieces; only the ends at
                    // the break light up.
                    let [a, b] = snapped_open(&border, &snap, k);
                    let pen = StrokeStyle::solid(width, line);
                    let pen = if glow > 0.0 { pen.glow(glow) } else { pen };
                    p.stroke(&a, &pen);
                    p.stroke(&b, &pen);
                    let len = a.length();
                    p.stroke(&a.trim(len - END, len), &tip);
                    p.stroke(&b.trim(0.0, END), &tip);
                }
                for c in snapped(&border, &snap, k).into_iter().filter(|_| !open) {
                    p.stroke(&c, &StrokeStyle::solid(width, line));
                    // The two ends light up in the accent as they part.
                    let len = c.length();
                    p.stroke(&c.trim(0.0, END), &tip);
                    p.stroke(&c.trim(len - END, len), &tip);
                }
                for &(font, glyph, pos) in &glyphs {
                    p.glyph(vo::scene::display::GlyphInstance {
                        xform: vo::scene::display::GlyphXform::IDENTITY,
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

/// An open edge broken at `snap.at` with progress `k`, as its two pieces:
/// they part by `k * snap.gap` and the ends at the break bend `k * snap.curl`
/// off the line, to the left of travel (up for an edge drawn left to right
/// on a y-down screen; a negative curl bends them the other way).
fn snapped_open(edge: &Contour, snap: &Snap, k: f32) -> [Contour; 2] {
    let len = edge.length();
    let k = k.clamp(0.0, 1.0);
    let half = 0.5 * snap.gap * k;
    let at = snap.at.clamp(0.0, len);
    let lift = snap.curl * k;
    let piece = |a: f32, b: f32, break_at_end: bool| {
        let span = (b - a).max(0.0);
        let hook = snap.hook.min(span);
        let n = (span.ceil() as usize).max(2);
        let pts: Vec<Point> = (0..=n)
            .filter_map(|i| {
                let s = a + span * i as f32 / n as f32;
                let (q, tan) = edge.at(s)?;
                let d = if break_at_end { b - s } else { s - a };
                let bend = if hook > 0.0 && d < hook {
                    let u = 1.0 - d / hook;
                    u * u
                } else {
                    0.0
                };
                Some(Point::new(
                    q.x + tan.y * lift * bend,
                    q.y - tan.x * lift * bend,
                ))
            })
            .collect();
        Contour::polyline(&pts, false)
    };
    [piece(0.0, at - half, true), piece(at + half, len, false)]
}

impl Popup {
    /// How a new word enters the type look: `normal`, or with an outline
    /// feature on, at the arrival ink with no rise.
    pub(crate) fn paths_word_enter(&self, normal: Transition) -> Transition {
        if self.look.on(WRITE) {
            // The pen's fill brings the ink up (SW-02), not a timed fade.
            Transition::NONE
        } else if self.paths_on() {
            Transition::fade().opacity(ARRIVAL_INK)
        } else {
            normal
        }
    }
}

/// Pen writings by word id, text and place, shared like the morphs.
type WriteKey = (u32, String, i32, i32, u32, u32);

fn write_cache() -> &'static Mutex<HashMap<WriteKey, Arc<Writing>>> {
    static C: std::sync::OnceLock<Mutex<HashMap<WriteKey, Arc<Writing>>>> =
        std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// One pen per popup (SW-01): its schedule and each word's slot in it, by
/// popup (`Popup::paths_popup`). Words join as they are first seen, so a
/// slot once given never changes.
struct PenBook {
    schedule: PenSchedule,
    slot: HashMap<u32, usize>,
    /// Each word's voice when it joined the pen, 0 to 1, frozen for its
    /// nib pressure (SW-05).
    voice: HashMap<u32, f32>,
    used: f64,
}

fn pens() -> &'static Mutex<HashMap<u64, PenBook>> {
    static C: std::sync::OnceLock<Mutex<HashMap<u64, PenBook>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

impl Popup {
    /// The pen writing each new word by its strokes (MO-80), over the live
    /// text, which arrives at the reading-line ink and fills as the pen
    /// passes. One pen for the popup, a word at a time (SW-01): a word the
    /// pen cannot start within 250 ms is swept in instead. The pen follows
    /// each letter's own centre lines in wet ink (SW-03): the text colour
    /// lifted toward white at the letter's stem width, drying back into the
    /// type 180 ms after the letter is done, with the dots, crossbars and
    /// marks written at the end of the word and only the nib light in the
    /// accent (`PathsText::ink` and `nib`).
    fn writing_overlay(
        &self,
        laid: &[Laid],
        text: &PathsText,
        w: f32,
        h: f32,
    ) -> Option<Element<Msg>> {
        if self.paths_reduced() {
            return None;
        }
        let style = &text.style;
        let now = self.extra.now;
        let wet = WetInk::new(text.ink, text.nib);
        self.pen_prefetch(style);
        let plays: Vec<(Arc<Writing>, PenSlot, f32)> = self
            .pen_slots(laid, style, |slot| wet.until(slot) >= now - 0.05)
            .into_iter()
            .map(|(_, writing, slot, voice)| (writing, slot, voice))
            .collect();
        if plays.is_empty() {
            return None;
        }
        let until = plays.iter().map(|p| wet.until(&p.1)).fold(0.0, f64::max);
        let draw = move |t: f64, r: Rect, p: &mut Painter| {
            p.layer(Affine::translate(r.x, r.y), 1.0, |p| {
                for (writing, slot, voice) in &plays {
                    writing.draw_wet_voiced(p, t, slot, &wet, *voice);
                }
            })
        };
        Some(kit::at(Element::canvas("stroke-write", until, draw), 0.0, 0.0).size(w, h))
    }

    /// Each word's live fill while the pen writes it (SW-02) in `text`, as
    /// fill tracks for `Typos::fill_track`: the word's letters stay at the
    /// floor until the pen reaches them and are full once it has written
    /// them. Empty without `stroke-write` and under reduced motion.
    pub(crate) fn pen_fills(&self, text: &PathsText) -> HashMap<u32, Vec<(f64, f32)>> {
        if !self.look.on(WRITE) || self.paths_reduced() {
            return HashMap::new();
        }
        let laid = self.paths_laid(text);
        let now = self.extra.now;
        self.pen_slots(&laid, &text.style, |slot| slot.end >= now - 0.05)
            .into_iter()
            .map(|(id, writing, slot, _)| (id, fill_keys(&writing, &slot)))
            .collect()
    }

    /// The laid words' writings, their slots with the popup's pen and
    /// their voices, for the slots `keep` takes. A word seen for the first
    /// time joins the pen here, so its slot and voice are the same whichever
    /// asks first.
    fn pen_slots(
        &self,
        laid: &[Laid],
        style: &TextStyle,
        keep: impl Fn(&PenSlot) -> bool,
    ) -> Vec<(u32, Arc<Writing>, PenSlot, f32)> {
        let size = style.size;
        let now = self.extra.now;
        let popup = self.paths_popup();
        let mut plays = Vec::new();
        let mut books = pens().lock().unwrap_or_else(|e| e.into_inner());
        if books.len() > 64 {
            books.retain(|_, b| now - b.used < 30.0);
        }
        let book = books.entry(popup).or_insert_with(|| PenBook {
            schedule: PenSchedule::new(PenParams {
                speed: PEN_SPEED * size,
                ..PenParams::default()
            }),
            slot: HashMap::new(),
            voice: HashMap::new(),
            used: now,
        });
        book.used = now;
        for word in laid {
            let known = book
                .slot
                .get(&word.id)
                .and_then(|&i| book.schedule.slot(i))
                .copied();
            if known.is_some_and(|x| !keep(&x)) {
                continue;
            }
            let writing = self.writing(word, style);
            let slot = match known {
                Some(x) => x,
                None => {
                    let born = self.paths_times(word).0;
                    let i = book.schedule.push(born, writing.length());
                    book.slot.insert(word.id, i);
                    book.voice.insert(word.id, self.pen_voice());
                    book.schedule.slots()[i]
                }
            };
            if !keep(&slot) {
                continue;
            }
            let voice = book.voice.get(&word.id).copied().unwrap_or(0.5);
            plays.push((word.id, writing, slot, voice));
        }
        plays
    }

    /// The voice a word joins the pen with: the level as it arrives, the
    /// loudest of the last six frames, 0 to 1.
    fn pen_voice(&self) -> f32 {
        let level = self
            .levels
            .iter()
            .rev()
            .take(6)
            .fold(0.0f64, |a, b| a.max(*b));
        level.clamp(0.0, 1.0) as f32
    }

    /// Queues the pen strokes of the text face's common letters on the
    /// centre-line worker, once per face.
    fn pen_prefetch(&self, style: &TextStyle) {
        static DONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        {
            let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
            if done.contains(&style.font) {
                return;
            }
            done.push(style.font);
        }
        let shaped = self.measure.layout(PEN_PREFETCH, style, None);
        let outline = WordOutline::new(self.measure.fonts(), &shaped, style.size, Point::ZERO);
        centreline::prefetch(self.measure.fonts(), &outline);
    }

    /// `word`'s pen strokes, built once and shared like the morphs. Live,
    /// a glyph whose centre lines are still being analysed is traced by its
    /// outline and the writing is built again next frame; a scripted popup
    /// (offline renders, tests) analyses them at once, so its frames repeat
    /// exactly.
    fn writing(&self, word: &Laid, style: &TextStyle) -> Arc<Writing> {
        let size = style.size;
        let key: WriteKey = (
            word.id,
            word.text.clone(),
            (word.x.0 * 4.0) as i32,
            (word.baseline * 4.0) as i32,
            style.font,
            size.to_bits(),
        );
        let mut c = write_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = c.get(&key) {
            return w.clone();
        }
        if c.len() > 256 {
            c.clear();
        }
        let mut outline = WordOutline::new(self.measure.fonts(), &word.shaped, size, Point::ZERO);
        // The laid glyph clusters index the joined text: rebase them onto
        // the word's own text.
        let base = word.shaped.glyphs.first().map_or(0, |g| g.cluster);
        for g in &mut outline.glyphs {
            g.cluster -= base;
        }
        let source = if self.scripted {
            StrokeSource::FontNow
        } else {
            StrokeSource::Font
        };
        let writing = Arc::new(Writing::with_source(
            self.measure.fonts(),
            &word.text,
            outline,
            0.15 * size,
            source,
        ));
        if !writing.pending() {
            c.insert(key, writing.clone());
        }
        writing
    }
}

/// A word's fill over scene time while the pen writes it in `slot`
/// (SW-02), as `(t, fill)` keys. Each letter's ink rises over the pen
/// distance from its first stroke to the end of its last base stroke (its
/// dots, crossbars and marks come at the end of the word), as the wet ink
/// dries into it; the fill's front is the sum of those letter shares. Pen
/// distance maps to time as the pen schedule shares it out. A swept word
/// fills evenly over its sweep.
fn fill_keys(writing: &Writing, slot: &PenSlot) -> Vec<(f64, f32)> {
    let n = writing.word.glyphs.len();
    let total = writing.length();
    if n == 0 || slot.swept || total <= 0.0 {
        return vec![(slot.start, 0.0), (slot.end, 1.0)];
    }
    // Each glyph's first stroke start, last base stroke end, last end.
    let mut span = vec![(f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY); n];
    for s in writing.strokes() {
        let Some(g) = span.get_mut(s.glyph) else {
            continue;
        };
        let end = s.start + s.contour.length();
        g.0 = g.0.min(s.start);
        if !s.delayed {
            g.1 = g.1.max(end);
        }
        g.2 = g.2.max(end);
    }
    // A glyph with no strokes is done with the one before it.
    let mut last = 0.0f32;
    let spans: Vec<(f32, f32)> = span
        .iter()
        .map(|&(b, base, end)| {
            let (b, e) = if b.is_finite() {
                (b, if base.is_finite() { base } else { end })
            } else {
                (last, last)
            };
            last = last.max(e);
            (b, e)
        })
        .collect();
    let front = |d: f32| -> f32 {
        spans
            .iter()
            .map(|&(b, e)| {
                if e > b {
                    ((d - b) / (e - b)).clamp(0.0, 1.0)
                } else if d >= b {
                    1.0
                } else {
                    0.0
                }
            })
            .sum()
    };
    let mut at: Vec<f32> = spans.iter().flat_map(|&(b, e)| [b, e]).collect();
    at.extend([0.0, total]);
    at.retain(|d| (0.0..=total).contains(d));
    at.sort_by(f32::total_cmp);
    at.dedup();
    let dur = slot.end - slot.start;
    at.into_iter()
        .map(|d| {
            let t = slot.start + dur * f64::from(d / total);
            (t, (front(d) / n as f32).clamp(0.0, 1.0))
        })
        .collect()
}

/// When each word was first seen and when its text last changed, by popup
/// and word. A view sees a word the moment its message lands, so the first
/// view that lays it out dates it.
type SeenKey = (u64, u32);

struct Seen {
    text: String,
    born: f64,
    changed: Option<f64>,
}

fn seen() -> &'static Mutex<HashMap<SeenKey, Seen>> {
    static C: std::sync::OnceLock<Mutex<HashMap<SeenKey, Seen>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

impl Popup {
    /// One popup: when it opened, its theme and its features, so the
    /// station's tiles keep apart.
    fn paths_popup(&self) -> u64 {
        self.look.features.iter().flat_map(|f| f.bytes()).fold(
            self.shown_at.to_bits() ^ (self.look.theme as u64).rotate_left(48),
            |h, b| h.rotate_left(5) ^ b as u64,
        )
    }

    /// When `w` came and when its text last changed after that.
    fn paths_times(&self, w: &Laid) -> (f64, Option<f64>) {
        let now = self.extra.now;
        let popup = self.paths_popup();
        let mut map = seen().lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 4096 {
            map.retain(|_, s| now - s.changed.unwrap_or(s.born) < 30.0);
        }
        let s = map.entry((popup, w.id)).or_insert_with(|| Seen {
            text: w.text.clone(),
            born: now,
            changed: None,
        });
        if s.text != w.text {
            s.text = w.text.clone();
            s.changed = Some(now);
        }
        (s.born, s.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vo::scene::text::ShapedLayout;
    use vo::scene::typo::Typo;

    fn writing(text: &str) -> Writing {
        let fonts = vo::FontRegistry::new();
        let id = fonts.asset("grotesk").unwrap();
        let shaped =
            ShapedLayout::new(fonts.clone()).layout(text, &TextStyle::new(id, 40.0, 48.0), None);
        let word = WordOutline::new(&fonts, &shaped, 40.0, Point::ZERO);
        Writing::with_source(&fonts, text, word, 6.0, StrokeSource::FontNow)
    }

    #[test]
    fn the_pen_fill_follows_the_pen_letter_by_letter() {
        let w = writing("write");
        let n = w.word.glyphs.len();
        let slot = PenSlot {
            arrive: 1.0,
            start: 1.1,
            end: 1.35,
            swept: false,
        };
        let keys = fill_keys(&w, &slot);
        // Empty before the pen, full when it is done, only rising between.
        assert_eq!(keys.first().copied(), Some((1.1, 0.0)));
        let last = *keys.last().unwrap();
        assert!(
            (last.0 - 1.35).abs() < 1e-9 && (last.1 - 1.0).abs() < 1e-6,
            "{keys:?}"
        );
        for k in keys.windows(2) {
            assert!(k[1].0 >= k[0].0 && k[1].1 >= k[0].1 - 1e-6, "{keys:?}");
        }
        // When the pen is done with the first letter's base strokes, that
        // letter is full and the last is still at the floor.
        let first = w
            .strokes()
            .filter(|s| s.glyph == 0 && !s.delayed)
            .map(|s| s.start + s.contour.length());
        let d = first.fold(0.0f32, f32::max);
        let t = slot.start + (slot.end - slot.start) * f64::from(d / w.length());
        let at = |t: f64| {
            let i = keys.partition_point(|k| k.0 <= t);
            if i == 0 {
                return keys[0].1;
            }
            if i == keys.len() {
                return last.1;
            }
            let (a, b) = (keys[i - 1], keys[i]);
            a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0)) as f32
        };
        let v = Typo::REST.fill(at(t));
        assert!(
            (v.letter_ink_over(0, n, PEN_FLOOR) - 1.0).abs() <= 0.02,
            "{}",
            v.fill
        );
        assert!(
            (v.letter_ink_over(n - 1, n, PEN_FLOOR) - 0.70).abs() <= 0.02,
            "{}",
            v.fill
        );
    }

    #[test]
    fn a_swept_word_fills_over_its_sweep() {
        let w = writing("swept");
        let slot = PenSlot {
            arrive: 1.0,
            start: 1.0,
            end: 1.08,
            swept: true,
        };
        assert_eq!(fill_keys(&w, &slot), vec![(1.0, 0.0), (1.08, 1.0)]);
    }

    /// Bounds of the morph's glyph outlines at `t`, from their sampled points.
    fn morph_bounds(m: &GlyphMorph, t: f32) -> [f32; 4] {
        let mut b = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for i in 0..m.word.glyphs.len() {
            for part in m.sample(i, t) {
                for seg in &part.contour.segs {
                    for p in [seg.p0, seg.p2] {
                        b = [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)];
                    }
                }
            }
        }
        b
    }

    #[test]
    fn the_outlines_sit_on_the_letters_of_the_style_they_are_given() {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let cfg = OsdConfig {
            theme: OsdTheme::Type,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        look.features = [MORPH].iter().map(|f| f.to_string()).collect();
        let mut popup = Popup::new(look.clone(), &fonts);
        popup.shown = vec![
            Word {
                id: 1,
                text: "Quill".into(),
            },
            Word {
                id: 2,
                text: "ink".into(),
            },
        ];
        let own = PathsText::of(&look);
        let serif = fonts.asset("serif").unwrap();
        assert_ne!(serif, own.style.font);
        let other = PathsText {
            style: TextStyle::new(serif, 1.6 * own.style.size, 1.6 * own.style.line_height),
            ..own
        };
        let bounds = |text: &PathsText| {
            let laid = popup.paths_laid(text);
            let m = popup.paths_morph(&laid[0], &text.style, Source::Dot);
            assert!(m.word.glyphs.iter().all(|g| g.font == text.style.font));
            morph_bounds(&m, 1.0)
        };
        // The first word as the live text shapes it in the other style.
        let joined = "Quill ink";
        let shaped =
            ShapedLayout::new(fonts.clone()).layout(joined, &other.style, Some(other.width));
        let first = ShapedText {
            glyphs: shaped
                .glyphs
                .iter()
                .filter(|g| g.cluster < 5)
                .copied()
                .collect(),
            ..Default::default()
        };
        let ink = WordOutline::new(&fonts, &first, other.style.size, Point::ZERO).ink;
        let want = [ink.x, ink.y, ink.x + ink.w, ink.y + ink.h];
        let got = bounds(&other);
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g - w).abs() <= 1.0,
                "outline bounds {got:?}, shaped glyph bounds {want:?}"
            );
        }
        // The look's own style lays the word elsewhere: the style is what moved it.
        let mine = bounds(&own);
        assert!(
            mine.iter().zip(got).any(|(a, b)| (a - b).abs() > 1.0),
            "{mine:?} {got:?}"
        );
    }
}
