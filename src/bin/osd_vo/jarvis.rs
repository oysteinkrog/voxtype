//! The `jarvis` look of the vo popup: a holographic HUD in cyan. A ring of
//! arcs, tick marks and thin segments sits round a radial waveform with the
//! microphone in its middle, small labels read the level and peak, and the
//! transcript sits centred in a chamfered frame with a status pill under it.
//!
//! The ring is one canvas drawn from the frame time: its segments and ticks
//! turn slowly while listening and ease to a stop once the recording stops;
//! the radial bars carry the level history, the newest level at the top and
//! older ones running down both sides. Everything traces itself on when the
//! popup opens. While the final text is made a progress arc with a light head
//! runs round the ring; when it lands a bright ring closes and a check draws
//! in the middle.
//!
//! Geometry is laid out in the pixels of the concept image and scaled so the
//! text frame is as wide as the popup.

use super::*;

use std::sync::OnceLock;

use std::f32::consts::TAU;
use vo::scene::ambient::{Loop, LoopProp};
use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::Point;
use vo::scene::path::{icons, make, Contour, Morph, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo::scene::{Bands, Spectrum, Ticks};

/// Width of the text frame in the concept, in concept pixels.
const CONCEPT_W: f32 = 677.0;
/// Room round the popup for glows, in logical pixels.
const MARGIN: f32 = 14.0;
/// Ring centre below the top of the HUD, in concept pixels.
const CENTRE_Y: f32 = 232.0;
/// Top of the text frame below the ring centre, in concept pixels.
const FRAME_DROP: f32 = 210.0;
/// Text line pitch and the frame's padding above and below the lines.
const LINE: f32 = 48.0;
const TEXT_PX: f32 = 38.5;
const FRAME_PAD_TOP: f32 = 33.0;
const FRAME_PAD_BOTTOM: f32 = 33.0;
const FRAME_CHAMFER: f32 = 38.0;
const PILL_GAP: f32 = 4.0;
const PILL_H: f32 = 54.0;
const PILL_END: f32 = 20.0;

/// Seconds for an outline to trace itself on.
const OPEN_SECS: f64 = 0.45;
/// After the recording stops, the ring keeps drawing this many times 0.6 s
/// while the bars die down.
const REST_SECS: f64 = 0.6;
/// Seconds of level history round each half of the bar ring.
const HISTORY_SECS: f32 = 0.6;
/// Level-history samples on each side, when the ring has no signal bus.
const HALF_BARS: usize = 46;
/// Bars round the spectrum ring: about one per mel band on each side, so
/// neighbouring bars differ instead of blending.
const SPECTRUM_BARS: u32 = 48;
/// Seconds for the done ring to close.
const CLOSE_SECS: f64 = 0.3;

const CYAN: u32 = 0x35e6ff;
const BRIGHT: u32 = 0x9ef6ff;
const MID: u32 = 0x1e9fd4;
const DIM: u32 = 0x0e5e86;
const INK: u32 = 0x03111d;
const TEXT: u32 = 0xe2f7ff;
const SLATE: u32 = 0x58809a;
/// The tips of the loudest bars, near white.
const HOT: u32 = 0xe4fdff;

fn hex(rgb: u32, a: f32) -> Color {
    Color::hex(rgb, a)
}

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

/// An element placed at `(x, y)` in its parent, out of the flow.
fn at<M>(e: Element<M>, x: f32, y: f32) -> Element<M> {
    e.layout(|s| {
        s.position = taffy::Position::Absolute;
        s.inset = taffy::Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        };
    })
}

/// Fonts for the transcript, the captions and the status.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    caption: u32,
    status: u32,
}

/// Barlow, a grotesk close to the concept's type, or the look's own sans
/// when it is not installed.
fn fonts(look: &Look) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |family: &str| look.fonts.add_system(family).unwrap_or(SANS);
        Fonts {
            text: add("Barlow:regular"),
            caption: add("Barlow Semi Condensed:medium"),
            status: add("Barlow:medium"),
        }
    })
}

/// Sizes in logical pixels, from the concept's.
#[derive(Clone, Copy)]
struct Geo {
    k: f32,
    w: f32,
    cx: f32,
    cy: f32,
    frame_top: f32,
}

impl Geo {
    fn new(look: &Look) -> Geo {
        let k = look.width / CONCEPT_W;
        let cy = MARGIN + CENTRE_Y * k;
        Geo {
            k,
            w: look.width,
            cx: MARGIN + look.width / 2.0,
            cy,
            frame_top: cy + FRAME_DROP * k,
        }
    }

    fn frame_h(&self, lines: u32) -> f32 {
        (FRAME_PAD_TOP + lines as f32 * LINE + FRAME_PAD_BOTTOM) * self.k
    }

    fn surface(&self) -> Size {
        let h = self.frame_top
            + self.frame_h(swiss::MAX_LINES as u32)
            + (PILL_GAP + PILL_H) * self.k
            + MARGIN;
        Size::new((self.w + 2.0 * MARGIN).ceil(), h.ceil())
    }
}

/// State only the jarvis look keeps.
#[derive(Default)]
pub struct Extra {
    /// When the dictation ended without a final text.
    ended_at: Option<f64>,
    /// The transcript's words when `lines` were made.
    words: Vec<Word>,
    /// The newest lines of the transcript, each with its width.
    lines: Vec<(Vec<Word>, f32)>,
}

/// Words laid out at most, from a start that only moves in steps of half
/// this, so the line breaks stay put while words arrive.
const LAYOUT_WORDS: usize = 120;

/// What the ring canvases draw, fixed for one view.
#[derive(Clone)]
struct Hud {
    k: f32,
    centre: Point,
    shown_at: f64,
    /// When the rings start to come to rest.
    rest_at: Option<f64>,
    final_at: Option<f64>,
    /// The level for the meter, 0 to 1.
    level: f32,
    wave: Arc<[f32]>,
    newest_at: f64,
    rate: f32,
}

impl Hud {
    /// How far an outline that starts tracing `delay` seconds after the open
    /// has got, 0 to 1.
    fn open(&self, t: f64, delay: f64) -> f32 {
        ease_out((t - self.shown_at - delay) / OPEN_SECS)
    }

    /// The level shown at `age` seconds before now, 0 to 1.
    fn level_at(&self, t: f64, age: f32) -> f32 {
        let n = self.wave.len();
        if n == 0 {
            return 0.0;
        }
        let i = (n - 1) as f32 - ((self.newest_at - t) as f32 + age) * self.rate;
        if i < 0.0 {
            return 0.0;
        }
        let i = i.min((n - 1) as f32);
        let (a, b) = (i.floor() as usize, i.ceil() as usize);
        let f = i - a as f32;
        self.wave[a] * (1.0 - f) + self.wave[b] * f
    }

    fn arc(&self, r: f32, from_deg: f32, to_deg: f32) -> Contour {
        make::arc(
            self.centre,
            r * self.k,
            from_deg.to_radians(),
            (to_deg - from_deg).to_radians(),
        )
    }

    /// A point `r` concept pixels from the centre at `deg` degrees.
    fn polar(&self, r: f32, deg: f32) -> Point {
        let a = deg.to_radians();
        Point::new(
            self.centre.x + r * self.k * a.cos(),
            self.centre.y + r * self.k * a.sin(),
        )
    }

    fn spoke(&self, r0: f32, r1: f32, deg: f32) -> Contour {
        Contour::polyline(&[self.polar(r0, deg), self.polar(r1, deg)], false)
    }

    /// The parts of the ring that do not turn: the disc, the outer arcs,
    /// the thin rings, the meters, the mic ring and the done rings.
    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        // The dark disc behind the ring.
        let o = self.open(t, 0.0);
        p.shape(ShapeInstance::circle(
            self.centre,
            236.0 * k,
            Paint::solid(hex(INK, 0.80 * o)),
        ));
        let trace = |c: Contour, o: f32| -> Contour {
            let len = c.length();
            c.trim(0.0, len * o)
        };
        let glow = |w: f32, c: u32, a: f32| StrokeStyle::solid(w * k, hex(c, a)).glow(3.0 * k);

        // Outer arcs, their spurs and the crosshair ticks: fixed.
        let o = self.open(t, 0.0);
        for (a, b) in [(142.0, 222.0), (-42.0, 38.0)] {
            p.stroke(&trace(self.arc(245.0, a, b), o), &glow(1.6, CYAN, 0.9));
        }
        for deg in [210.0, -28.0] {
            p.stroke(
                &trace(self.spoke(205.0, 238.0, deg), o),
                &glow(1.2, MID, 0.8),
            );
        }
        for deg in [0.0, 180.0] {
            p.stroke(
                &trace(self.spoke(234.0, 256.0, deg), o),
                &glow(1.2, CYAN, 0.8),
            );
        }

        // The bright ticks at 12 and 6 o'clock, and the thin full rings.
        let o = self.open(t, 0.05);
        for deg in [-90.0, 90.0] {
            p.stroke(
                &trace(self.spoke(199.0, 219.0, deg), o),
                &glow(3.0, CYAN, 1.0),
            );
        }
        let o = self.open(t, 0.1);
        p.stroke(
            &trace(make::ring(self.centre, 188.0 * k), o),
            &StrokeStyle::solid(1.0 * k, hex(MID, 0.75)),
        );
        p.stroke(
            &trace(make::ring(self.centre, 165.0 * k), o),
            &StrokeStyle::solid(1.0 * k, hex(DIM, 0.8)),
        );

        // The level meter on the left, lit from the bottom up, and its quiet
        // twin on the right.
        let o = self.open(t, 0.2);
        let lit = (self.level * 22.0).round() as usize;
        for i in 0..22 {
            let deg = 150.0 + i as f32 * 1.4;
            let a = if i < lit { 0.95 } else { 0.25 };
            p.stroke(
                &self.spoke(206.0, 216.0, deg),
                &StrokeStyle::solid(2.0 * k, hex(CYAN, a * o)),
            );
        }
        for i in 0..14 {
            let deg = 22.0 + i as f32 * 1.8;
            p.stroke(
                &self.spoke(208.0, 214.0, deg),
                &StrokeStyle::solid(1.4 * k, hex(DIM, 0.6 * o)),
            );
        }

        // The ring round the microphone.
        let o = self.open(t, 0.1);
        p.stroke(
            &trace(make::ring(self.centre, 35.0 * k), o),
            &glow(1.5, CYAN, 0.9),
        );

        // Done: two bright rings close from the top.
        if let Some(f) = self.final_at {
            for (r, delay, w) in [(178.0, 0.0, 3.0), (122.0, 0.08, 2.0)] {
                let d = ease_out((t - f - delay) / CLOSE_SECS);
                p.stroke(
                    &trace(make::ring(self.centre, r * k), d),
                    &StrokeStyle::solid(w * k, hex(CYAN, 1.0)).glow(5.0 * k),
                );
            }
        }
    }

    /// The bright segments that turn at `speed` degrees a second, drawn
    /// unturned round `self.centre`; the element they sit in turns them.
    fn draw_segments(&self, speed: i32, t: f64, p: &mut Painter) {
        let o = self.open(t, 0.15);
        for (r, a, b, w, s, c) in SEGMENTS {
            if s == speed {
                let arc = self.arc(r, a, b);
                let len = arc.length();
                p.stroke(
                    &arc.trim(0.0, len * o),
                    &StrokeStyle::solid(w * self.k, hex(c, 1.0)).glow(5.0 * self.k),
                );
            }
        }
    }

    /// The bar energies for the spectrum ring, newest first: the last
    /// moments of the level, shaped by a smooth spectrum that drifts round
    /// the ring and a little grain per bar, so the ring reads as a spectrum
    /// rather than two mirrored waveforms.
    fn bar_energies(&self, t: f64) -> Arc<[f32]> {
        let fade = self
            .rest_at
            .map_or(1.0, |r| (-((t - r).max(0.0)) / 0.15).exp() as f32);
        let tf = t as f32;
        (0..HALF_BARS)
            .map(|j| {
                let a = (-90.0 + (j as f32 + 0.5) * 180.0 / HALF_BARS as f32).to_radians();
                let age = j as f32 / HALF_BARS as f32 * HISTORY_SECS;
                let seed = (j as f32 * 12.9898 + 78.233).sin() * 43758.547;
                let grain = 0.55 + 0.45 * seed.fract().abs();
                let shape = 0.55
                    + 0.45
                        * (0.5 + 0.5 * (2.0 * a + 0.9 * tf).sin())
                        * (0.5 + 0.5 * (5.0 * a - 1.7 * tf).sin());
                (self.level_at(t, age) * fade * 1.5 * shape * grain).min(1.0)
            })
            .collect()
    }
}

/// The segment groups that turn and their speeds in degrees a second;
/// clockwise is positive.
const SEGMENT_SPEEDS: [i32; 3] = [12, -8, 5];

/// Bright segments: radius, from and to in degrees, width, the speed of the
/// group they turn with, colour.
const SEGMENTS: [(f32, f32, f32, f32, i32, u32); 5] = [
    (180.0, -73.0, -36.0, 5.0, 12, CYAN),
    (165.0, 102.0, 162.0, 3.0, -8, CYAN),
    (190.0, 29.0, 45.0, 6.0, 12, CYAN),
    (188.0, -118.0, -90.0, 2.0, 5, BRIGHT),
    (170.0, -8.0, 14.0, 2.0, -8, CYAN),
];

/// The tick ring's turn and the dotted ring's, degrees a second.
const TICKS_SPEED: f64 = -3.0;
const DOTS_SPEED: f64 = 6.0;

/// A steady turn at `deg_per_s`, as an ambient loop on rotation. It comes to
/// rest by itself when the view stops declaring it, and holds still under
/// reduced motion.
fn spin(deg_per_s: f64) -> Loop {
    let turn = Loop::spin(360.0 / deg_per_s.abs());
    Loop {
        amp: turn.amp.copysign(deg_per_s),
        ..turn
    }
}

/// The chamfered frame round the transcript in `r`, traced on by `o`.
fn draw_frame(r: Rect, k: f32, o: f32, p: &mut Painter) {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let c = FRAME_CHAMFER * k;
    let notch = 10.0 * k;
    let n1 = x + 0.64 * w;
    let mid = x + w / 2.0;
    // Right half from the top middle round to the bottom middle; the left
    // half is its mirror.
    let right = |inset: f32| {
        let (y0, x1, y1) = (y + inset, x + w - inset, y + h - inset);
        vec![
            Point::new(mid, y0 + notch),
            Point::new(n1 - inset, y0 + notch),
            Point::new(n1 + 12.0 * k - inset, y0),
            Point::new(x1 - c, y0),
            Point::new(x1, y0 + c),
            Point::new(x1, y1 - c),
            Point::new(x1 - c, y1),
            Point::new(mid, y1),
        ]
    };
    let mirror = |pts: &[Point]| -> Vec<Point> {
        pts.iter()
            .map(|q| Point::new(2.0 * mid - q.x, q.y))
            .collect()
    };
    let outer = right(0.0);
    let whole: Vec<Point> = outer
        .iter()
        .copied()
        .chain(mirror(&outer).into_iter().rev())
        .collect();
    p.fill(
        [&Contour::polyline(&whole, true)],
        &StrokeStyle::solid(1.0, hex(INK, 0.82 * o)),
    );
    let trace = |pts: &[Point]| {
        let c = Contour::polyline(pts, false);
        let len = c.length();
        c.trim(0.0, len * o)
    };
    let line = StrokeStyle::solid(1.1 * k, hex(CYAN, 0.85)).glow(1.5 * k);
    let inner = StrokeStyle::solid(1.0 * k, hex(DIM, 0.7));
    let inset = right(6.0 * k);
    for pts in [outer.clone(), mirror(&outer)] {
        p.stroke(&trace(&pts), &line);
    }
    for pts in [inset.clone(), mirror(&inset)] {
        p.stroke(&trace(&pts), &inner);
    }
    // Bright brackets over the four chamfers.
    let accent = StrokeStyle::solid(1.8 * k, hex(BRIGHT, 0.9 * o)).glow(3.0 * k);
    for (sx, sy) in [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        let cx = if sx > 0.0 { x + w } else { x };
        let cy = if sy > 0.0 { y } else { y + h };
        let pts = [
            Point::new(cx - sx * (c + 30.0 * k), cy),
            Point::new(cx - sx * c, cy),
            Point::new(cx, cy + sy * c),
        ];
        p.stroke(&Contour::polyline(&pts, false), &accent);
    }
    // Dash meters inside each side and a fine ladder outside the right.
    let dash = StrokeStyle::solid(3.0 * k, hex(CYAN, 0.85 * o));
    for i in 0..8 {
        let dy = y + h * (0.32 + 0.36 * i as f32 / 7.0);
        for x0 in [x + 11.0 * k, x + w - 17.0 * k] {
            p.stroke(
                &Contour::polyline(&[Point::new(x0, dy), Point::new(x0 + 6.0 * k, dy)], false),
                &dash,
            );
        }
    }
    let tick = StrokeStyle::solid(1.0 * k, hex(DIM, 0.8 * o));
    for i in 0..12 {
        let dy = y + h * (0.3 + 0.4 * i as f32 / 11.0);
        for (a, b) in [
            (x + w + 7.0 * k, x + w + 13.0 * k),
            (x - 13.0 * k, x - 7.0 * k),
        ] {
            p.stroke(
                &Contour::polyline(&[Point::new(a, dy), Point::new(b, dy)], false),
                &tick,
            );
        }
    }
}

/// The status pill with pointed ends in `r`, traced on by `o`.
fn draw_pill(r: Rect, k: f32, o: f32, p: &mut Painter) {
    let e = PILL_END * k;
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let mid = x + w / 2.0;
    let right = [
        Point::new(mid, y),
        Point::new(x + w - e, y),
        Point::new(x + w, y + h / 2.0),
        Point::new(x + w - e, y + h),
        Point::new(mid, y + h),
    ];
    let left: Vec<Point> = right
        .iter()
        .map(|q| Point::new(2.0 * mid - q.x, q.y))
        .collect();
    let whole: Vec<Point> = right
        .iter()
        .copied()
        .chain(left.iter().rev().copied())
        .collect();
    p.fill(
        [&Contour::polyline(&whole, true)],
        &StrokeStyle::solid(1.0, hex(INK, 0.85 * o)),
    );
    let style = StrokeStyle::solid(1.1 * k, hex(CYAN, 0.8)).glow(1.5 * k);
    for pts in [&right[..], &left[..]] {
        let c = Contour::polyline(pts, false);
        let len = c.length();
        p.stroke(&c.trim(0.0, len * o), &style);
    }
}

/// The transcript's type: Barlow, scaled with the popup.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = look.width / CONCEPT_W;
    TextStyle::new(fonts(look).text, TEXT_PX * k, LINE * k)
}

/// The centre of the newest text line below the surface's top with `lines`
/// lines: the frame grows downward from under the ring, so it moves down a
/// line pitch per line. The reading-line anchor shifts the whole HUD to
/// make up for it, so on screen the HUD rises instead.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    let g = Geo::new(look);
    g.frame_top + (FRAME_PAD_TOP + (lines.max(1) as f32 - 0.5) * LINE) * g.k
}

/// Width the transcript wraps at: the frame less its side meters and some air.
pub(crate) fn text_width(look: &Look) -> f32 {
    look.width - 2.0 * 64.0 * look.width / CONCEPT_W
}

impl Popup {
    pub(crate) fn jarvis_surface(look: &Look) -> Size {
        Geo::new(look).surface()
    }

    /// Notes when a dictation ended without a final text, and breaks the
    /// transcript into lines when its words change.
    pub(crate) fn sync_jarvis(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Jarvis {
            return;
        }
        // The recording stopped and nothing is being finished. That is a
        // cancel, or the moment before the final pass writes its status, so
        // it is forgotten as soon as the final pass shows up.
        if self.extra.finish_at.is_some() || self.extra.final_at.is_some() {
            self.jarvis.ended_at = None;
        } else if self.ended() && self.jarvis.ended_at.is_none() {
            self.jarvis.ended_at = Some(t);
        }
        if self.model.words() != self.jarvis.words.as_slice() {
            let words = self.model.words().to_vec();
            let step = LAYOUT_WORDS / 2;
            let start = words.len().saturating_sub(LAYOUT_WORDS).div_ceil(step) * step;
            let mut lines = self.jarvis_lines(&words[start.min(words.len())..]);
            let keep = lines.len().saturating_sub(swiss::MAX_LINES as usize);
            lines.drain(..keep);
            self.jarvis.lines = lines;
            self.jarvis.words = words;
        }
    }

    /// The loudest level of the last whole half second, so the figures
    /// change twice a second and can be read between rolls.
    fn held_level(&self) -> f32 {
        let b = (FRAME_HZ / 2) as usize;
        let n = self.levels.len();
        let end = n.saturating_sub((self.received % b as u64) as usize);
        let start = end.saturating_sub(b);
        self.levels.range(start..end).fold(0.0f64, |a, l| a.max(*l)) as f32
    }

    pub(crate) fn jarvis_view(&self) -> Element<Msg> {
        let g = Geo::new(&self.look);
        let size = g.surface();
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        let status = self.status();
        let done = self.extra.final_at.is_some();
        let exit = if done {
            Transition::fade().offset(0.0, -8.0)
        } else {
            Transition::fade().scale(0.96)
        };
        let hud = at(Element::layer("hud"), 0.0, 0.0)
            .size(size.w, size.h)
            .bloom(0.2, 6.0)
            .presence(Transition::fade().opacity(1.0), exit)
            .child(self.jarvis_ring(&g, &status, size))
            .child(self.jarvis_center(&g, &status))
            .child(self.jarvis_labels(&g, &status))
            .child(self.jarvis_frame(&g, &status))
            .child(self.jarvis_pill(&g, &status));
        root.child(hud)
    }

    fn jarvis_ring(&self, g: &Geo, status: &StatusLine, size: Size) -> Element<Msg> {
        let finishing = status.finishing && !status.loading;
        let rest_at = self.extra.finish_at.or(self.jarvis.ended_at).or_else(|| {
            // Finishing without a stop seen yet: rest from now.
            finishing.then_some(self.extra.now)
        });
        let hud = Hud {
            k: g.k,
            centre: Point::new(g.cx, g.cy),
            shown_at: self.shown_at,
            rest_at,
            final_at: self.extra.final_at,
            level: self.held_level(),
            wave: self.wave.clone(),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
        };
        let until = self.jarvis_until();
        // The turning parts each sit in a square round the ring centre and
        // turn on an ambient loop while listening. Once the recording stops
        // the view leaves the loops out and they come to rest; under reduced
        // motion they hold still.
        let live = rest_at.is_none();
        let half = 222.0 * g.k;
        let local = Hud {
            centre: Point::new(half, half),
            ..hud.clone()
        };
        let mut layer = at(Element::layer("ring-layer"), 0.0, 0.0)
            .size(size.w, size.h)
            // A stop with no final pass dims the HUD.
            .opacity(if self.jarvis.ended_at.is_some() {
                0.35
            } else {
                1.0
            });
        let square = |e: Element<Msg>| at(e, g.cx - half, g.cy - half).size(2.0 * half, 2.0 * half);
        let turn = |e: Element<Msg>, speed: f64| {
            if live {
                e.ambient(LoopProp::Rotation, spin(speed))
            } else {
                e
            }
        };
        let draw_on = Transition::NONE.trim(0.0, 0.0);
        // The tick rings on the top and bottom arcs, every fifth tick longer.
        for (i, start) in [220.0f32, 40.0].into_iter().enumerate() {
            let ticks = Ticks::new(34, 7.0 * g.k, 1.1 * g.k, hex(MID, 0.55))
                .major(5, 11.0 * g.k, 1.1 * g.k, hex(MID, 0.85))
                .align(-1.0);
            let e = Element::polar(
                format!("ticks{i}"),
                2.0 * half,
                2.0 * half,
                215.0 * g.k,
                start.to_radians(),
                100f32.to_radians(),
            )
            .ticks(ticks)
            .presence(draw_on, Transition::fade());
            layer = layer.child(turn(square(e), TICKS_SPEED));
        }
        // The dotted ring: dot-long ticks.
        let dots = Element::polar("dots", 2.0 * half, 2.0 * half, 142.0 * g.k, 0.0, TAU)
            .ticks(Ticks::new(90, 0.01, 1.8 * g.k, hex(MID, 0.7)))
            .presence(draw_on, Transition::fade());
        layer = layer.child(turn(square(dots), DOTS_SPEED));
        for speed in SEGMENT_SPEEDS {
            let local = local.clone();
            let mut e = square(Element::canvas(
                format!("spin{speed}"),
                self.shown_at + 1.0,
                move |t, _, p| local.draw_segments(speed, t, p),
            ));
            e = turn(e, speed as f64);
            if !live {
                e = e.opacity(0.65);
            }
            layer = layer.child(e);
        }
        // The spectrum: the last moments of the level round the ring, the
        // newest at the top on both sides. It leaves when the check lands.
        if self.extra.final_at.is_none() {
            let resting = rest_at.is_some();
            let r = 160.0 * g.k;
            // The mel bands from the signal bus, low bands at the top on both
            // sides; the level history when the daemon sends no voice frames.
            let bands = if self.signals.get().is_empty() {
                Bands::Fixed(hud.bar_energies(self.extra.now))
            } else {
                Bands::Signals {
                    lo: 0,
                    hi: 23,
                    attack: 0.015,
                    release: 0.18,
                }
            };
            let ring = |low: Color, high: Color, width: f32| {
                Spectrum::new(low, high)
                    .bands(bands.clone())
                    .count(SPECTRUM_BARS)
                    .mirror()
                    .ring(62.0 / 160.0, 0.95)
                    .rest(3.0 * g.k)
                    .width(width)
                    .curve(1.6)
                    // Fixed per-bar variation, the same seed on both rings so
                    // the cores match their bars. No drift, so the scene sleeps
                    // when the bus is quiet.
                    .jitter(0.4, 7)
            };
            let place = |key: &'static str, spectrum: Spectrum| {
                at(Element::spectrum_ring(key, spectrum), g.cx - r, g.cy - r)
                    .size(2.0 * r, 2.0 * r)
                    .presence(Transition::fade(), Transition::fade())
            };
            // Saturated cyan bars with their glow, and while listening a thin
            // near-white core that only the loud bars light.
            let (low, high) = if resting {
                (hex(SLATE, 0.12), hex(SLATE, 0.4))
            } else {
                (hex(CYAN, 0.5), hex(CYAN, 1.0))
            };
            layer = layer.child(place("spectrum", ring(low, high, 0.3).glow(1.2 * g.k, 1.2)));
            if !resting {
                layer = layer.child(place(
                    "spectrum-core",
                    ring(hex(HOT, 0.0), hex(HOT, 0.9), 0.1),
                ));
            }
        }
        let ring = at(
            Element::canvas("ring", until, move |t, _, p| hud.draw(t, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h);
        layer = layer.child(ring);
        if self.shows_progress(status) && self.extra.final_at.is_none() {
            layer = layer.child(self.jarvis_progress(g));
        }
        layer
    }

    /// When the rings, the frame and the pill stop moving: never while
    /// listening, since the rings turn; a while after the rings come to
    /// rest or the check lands. The frame and the pill grow with the text,
    /// so they share this rather than stop after the open.
    fn jarvis_until(&self) -> f64 {
        let rest_at = self.extra.finish_at.or(self.jarvis.ended_at);
        match (self.extra.final_at, rest_at) {
            (Some(f), _) => f + 1.0,
            (None, Some(r)) => r + 6.0 * REST_SECS,
            _ => f64::INFINITY,
        }
    }

    /// A bright arc on the thin ring that fills with the progress, with a
    /// light head running on it.
    fn jarvis_progress(&self, g: &Geo) -> Element<Msg> {
        let r = 188.0 * g.k;
        let side = 2.0 * r + 8.0;
        let outline = Arc::new(Outline {
            contours: vec![make::ring(Point::new(side / 2.0, side / 2.0), r)],
        });
        let fraction = self
            .files
            .fraction
            .map_or(0.0, |f| f.clamp(0.0, 1.0) as f32);
        at(
            Element::stroke_path(
                "progress",
                outline,
                Rect::new(0.0, 0.0, side, side),
                StrokeStyle::solid(5.0 * g.k, hex(CYAN, 0.95)).glow(3.0 * g.k),
            ),
            g.cx - side / 2.0,
            g.cy - side / 2.0,
        )
        .size(side, side)
        .trim(0.0, fraction)
        .light_head(0.08, 0.7, hex(BRIGHT, 1.0), 3.0 * g.k)
        .motion(Prop::Trim, Spring::critical_to_98(0.28))
        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
    }

    /// The microphone, the stop square or the check in the middle.
    fn jarvis_center(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let side = 34.0 * g.k;
        let place = |e: Element<Msg>, s: f32| at(e.size(s, s), g.cx - s / 2.0, g.cy - s / 2.0);
        let slot = at(Element::layer("center"), 0.0, 0.0);
        if self.extra.final_at.is_some() {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot.child(place(
                Element::stroke_path(
                    "check",
                    check.clone(),
                    view,
                    StrokeStyle::solid(3.0 * g.k, hex(BRIGHT, 1.0)).glow(6.0 * g.k),
                )
                .motion(Prop::Trim, Spring::critical_to_98(0.52))
                .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                side * 1.5,
            ));
        }
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 64))
        });
        let stopped = status.finishing && !status.loading;
        let s = if stopped { side * 1.25 } else { side };
        slot.child(place(
            Element::morph(
                "mic",
                morph.clone(),
                view,
                StrokeStyle::solid(1.8 * g.k, hex(CYAN, 1.0)).glow(2.0 * g.k),
            )
            .morph_to(if stopped { 1.0 } else { 0.0 })
            .motion(Prop::Morph, Spring::critical_to_98(0.3))
            .presence(Transition::fade(), Transition::fade().scale(0.6)),
            s,
        ))
    }

    /// The two small readouts left and right of the ring: a caption and a
    /// figure whose digits roll.
    fn jarvis_labels(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        let f = fonts(&self.look);
        let k = g.k;
        let level = self.held_level();
        let percent = |v: f32| format!("{}%", (v * 100.0).round() as i32);
        let fraction = self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32);
        let (left, right) = if self.extra.final_at.is_some() {
            (
                ("WORDS", self.word_count.to_string()),
                ("TIME", self.clock.clone()),
            )
        } else if status.loading {
            (
                ("MODEL", fraction.map_or("--".into(), percent)),
                ("TIME", self.clock.clone()),
            )
        } else if status.finishing && self.jarvis.ended_at.is_none() {
            let left_secs = status
                .detail
                .split(" s of audio")
                .next()
                .and_then(|s| s.split_whitespace().last())
                .filter(|s| s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty())
                .map(|s| format!("{s} s"));
            (
                ("FINAL PASS", fraction.map_or("--".into(), percent)),
                match left_secs {
                    Some(s) => ("AUDIO LEFT", s),
                    None => ("TIME", self.clock.clone()),
                },
            )
        } else {
            // The level is a peak on a -60 to 0 dBFS scale raised to 1.5.
            let db = -60.0 + 60.0 * level.max(0.0).powf(2.0 / 3.0);
            (
                ("AUDIO LEVEL", percent(level)),
                ("PEAK", format!("{} dB", db.round() as i32)),
            )
        };
        let readout =
            |key: &'static str, (caption, value): (&'static str, String), x: f32, y: f32| {
                let caption_el = Element::layer("caption-slot")
                    .size(110.0 * k, 13.0 * k)
                    .clip()
                    .child(
                        at(
                            Element::label(format!("caption-{caption}"), caption)
                                .text_style(
                                    TextStyle::new(f.caption, 10.5 * k, 13.0 * k).tracking(0.6 * k),
                                )
                                .color(hex(MID, 1.0)),
                            0.0,
                            0.0,
                        )
                        .presence(
                            Transition::fade().offset(0.0, 13.0 * k),
                            Transition::fade().offset(0.0, -13.0 * k),
                        ),
                    );
                let figure = digit_roll(
                    format!("value-{caption}"),
                    &value,
                    TextStyle::new(f.status, 21.0 * k, 26.0 * k),
                    hex(CYAN, 1.0),
                    Spring::critical_to_98(0.12),
                )
                .presence(Transition::fade(), Transition::fade());
                at(Element::column(key).gap(3.0 * k), x, y)
                    .child(caption_el)
                    .child(figure)
                    .presence(Transition::fade(), Transition::fade())
            };
        at(Element::layer("labels"), 0.0, 0.0)
            .child(readout("left", left, g.cx - 228.0 * k, g.cy - 66.0 * k))
            .child(readout("right", right, g.cx + 182.0 * k, g.cy - 60.0 * k))
            .opacity(1.0)
            .presence(Transition::fade(), Transition::fade())
    }

    /// The shown words broken into lines as the layout breaks them, each
    /// with its width.
    fn jarvis_lines(&self, words: &[Word]) -> Vec<(Vec<Word>, f32)> {
        let style = self.look.text_style();
        let mut text = String::new();
        let mut starts = Vec::with_capacity(words.len());
        for w in words {
            if !text.is_empty() {
                text.push(' ');
            }
            starts.push(text.len() as u32);
            text.push_str(&w.text);
        }
        let shaped = self
            .measure
            .layout(&text, &style, Some(self.look.text_width()));
        let line_of = |y: f32| (y / style.line_height).floor() as i64;
        let mut lines: Vec<(i64, Vec<Word>)> = Vec::new();
        for (i, w) in words.iter().enumerate() {
            let Some(first) = shaped.glyphs.iter().find(|g| g.cluster >= starts[i]) else {
                continue;
            };
            let line = line_of(first.pos.y);
            match lines.last_mut() {
                Some((l, words)) if *l == line => words.push(w.clone()),
                _ => lines.push((line, vec![w.clone()])),
            }
        }
        lines
            .into_iter()
            .map(|(_, words)| {
                let text = words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let width = self.measure.layout(&text, &style, None).size.w;
                (words, width)
            })
            .collect()
    }

    /// The chamfered frame with the centred transcript.
    fn jarvis_frame(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        let k = g.k;
        let rows = self.lines.max(1);
        let h = g.frame_h(rows);
        let shown_at = self.shown_at;
        let frame = at(
            Element::canvas("frame", self.jarvis_until(), move |t, r, p| {
                let o = ease_out((t - shown_at - 0.12) / OPEN_SECS);
                draw_frame(r, k, o, p)
            }),
            0.0,
            0.0,
        )
        .size(g.w, h)
        .motion(Prop::Size, Spring::SNAP);
        let style = self.look.text_style();
        let mut column = Element::column("lines")
            .align_items(AlignItems::CENTER)
            .width(g.w)
            .height(rows as f32 * style.line_height)
            .margin_edges(FRAME_PAD_TOP * k, 0.0, 0.0, 0.0);
        // Each line is keyed by its first word, so when the top line leaves
        // the others keep their nodes and move up.
        // The popup counts the lines it has room for; show that many.
        let skip = self.jarvis.lines.len().saturating_sub(rows as usize);
        for (words, width) in self.jarvis.lines.iter().skip(skip) {
            let key = words.first().map_or(0, |w| w.id);
            let w = width.ceil() + 2.0;
            let line = Element::layer(format!("line{key}"))
                .size(w, style.line_height)
                .presence(Transition::fade(), Transition::fade())
                .child(
                    at(Element::live_text("words", words), 0.0, 0.0)
                        .text_style(style)
                        .color(hex(TEXT, 1.0))
                        .text_halo(hex(CYAN, 0.45), 7.0 * k)
                        .size(w, style.line_height)
                        .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                        .motion(Prop::Offset, Spring::POP)
                        .word_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0 * k))
                        .word_change(Transition::fade().opacity(0.35).offset(0.0, -4.0 * k)),
                );
            column = column.child(line);
        }
        if self.jarvis.lines.is_empty() && status.loading {
            // Nothing heard yet while the model loads: say so in the frame.
            let what = status.detail.trim_start_matches(['·', ' ']);
            let what = what.split(',').next().unwrap_or("");
            column = column.child(
                Element::label(
                    "waiting",
                    format!("{} {what}", status.state).trim().to_string(),
                )
                .text_style(style)
                .color(hex(MID, 1.0))
                .presence(
                    Transition::fade(),
                    Transition::fade().offset(0.0, -16.0 * k),
                ),
            );
        }
        at(Element::column("frame-box"), MARGIN, g.frame_top)
            .size(g.w, h)
            .motion(Prop::Size, Spring::SNAP)
            .child(frame)
            .child(column)
    }

    /// The status pill under the frame: icon, state and detail.
    fn jarvis_pill(&self, g: &Geo, status: &StatusLine) -> Element<Msg> {
        let k = g.k;
        let f = fonts(&self.look);
        let shown_at = self.shown_at;
        let h = PILL_H * k;
        let outline = |w: f32| {
            at(
                Element::canvas("pill-frame", self.jarvis_until(), move |t, r, p| {
                    let o = ease_out((t - shown_at - 0.3) / OPEN_SECS);
                    draw_pill(r, k, o, p)
                }),
                0.0,
                0.0,
            )
            .size(w, h)
        };
        let done = self.extra.final_at.is_some();
        let (state, detail) = if done {
            ("Done".to_string(), format!("·  {} words", self.word_count))
        } else {
            (status.state.clone(), status.detail.trim().to_string())
        };
        let state_style = TextStyle::new(f.status, 20.0 * k, 26.0 * k);
        let detail_style = TextStyle::new(f.text, 17.0 * k, 26.0 * k);
        let state_w = self
            .measure
            .layout(&state, &state_style, None)
            .size
            .w
            .ceil();
        let label = Element::layer("state")
            .clip()
            .size(state_w + 2.0, 26.0 * k)
            .margin_edges(0.0, 0.0, 0.0, 10.0 * k)
            .child(
                at(
                    Element::label(format!("state-{state}"), state.as_str())
                        .text_style(state_style)
                        .color(hex(CYAN, 1.0)),
                    0.0,
                    0.0,
                )
                .presence(
                    Transition::fade().offset(0.0, 24.0 * k),
                    Transition::fade().offset(0.0, -24.0 * k),
                ),
            );
        let base = state
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
            << 6;
        let words: Vec<Word> = detail
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| Word {
                id: base | i as u32 & 63,
                text: w.to_string(),
            })
            .collect();
        let detail_w = self
            .measure
            .layout(&detail, &detail_style, None)
            .size
            .w
            .ceil();
        let detail_el = Element::live_text(format!("detail-{state}"), &words)
            .text_style(detail_style)
            .color(hex(CYAN, 0.78))
            .width(detail_w + 2.0)
            .height(26.0 * k)
            .margin_edges(0.0, 0.0, 0.0, 10.0 * k)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().offset(0.0, 6.0 * k))
            .word_change(Transition::fade().opacity(0.2).offset(0.0, -6.0 * k))
            .presence(Transition::fade(), Transition::fade());
        let (pad_l, pad_r, gap) = (30.0 * k, 34.0 * k, 10.0 * k);
        let w = pad_l + 22.0 * k + gap + state_w + 2.0 + gap + detail_w + 2.0 + pad_r;
        let row = Element::row("pill")
            .align_items(AlignItems::CENTER)
            .size(w, h)
            .pad_edges(0.0, pad_r, 0.0, pad_l)
            .child(outline(w))
            .child(self.jarvis_pill_icon(k, status))
            .child(label)
            .child(detail_el);
        let top = g.frame_top + g.frame_h(self.lines.max(1)) + PILL_GAP * k;
        at(Element::column("pill-box"), 0.0, top)
            .width(g.w + 2.0 * MARGIN)
            .align_items(AlignItems::CENTER)
            .child(row)
    }

    /// A radio dot while listening, a hollow ring while loading, finishing
    /// or stopped, and a check when done.
    fn jarvis_pill_icon(&self, k: f32, status: &StatusLine) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        let side = 22.0 * k;
        let slot = Element::layer("icon").size(side, side);
        let ring = at(Element::shape("ring", ShapeKind::Circle), 0.0, 0.0)
            .size(side, side)
            .fill(Color::TRANSPARENT)
            .border(hex(CYAN, 1.0), 1.6 * k)
            .glow(hex(CYAN, 1.0), 4.0 * k, 0.5);
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let inner = side * 0.56;
        let icon = |key: &'static str, outline: &Arc<Outline>| {
            at(
                Element::stroke_path(
                    key,
                    outline.clone(),
                    view,
                    StrokeStyle::solid(2.0 * k, hex(BRIGHT, 1.0)),
                )
                .size(inner, inner)
                .motion(Prop::Trim, Spring::critical_to_98(0.4))
                .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                (side - inner) / 2.0,
                (side - inner) / 2.0,
            )
        };
        if self.extra.final_at.is_some() {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot.child(ring).child(icon("check", check));
        }
        let dot = 11.0 * k;
        let filled = !status.finishing;
        slot.child(ring).child(
            at(
                Element::shape("dot", ShapeKind::Circle),
                (side - dot) / 2.0,
                (side - dot) / 2.0,
            )
            .size(dot, dot)
            .fill(hex(CYAN, if filled { 1.0 } else { 0.0 }))
            .glow(hex(CYAN, 1.0), 4.0 * k, if filled { 0.6 } else { 0.0 }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spin_turns_once_per_period_in_its_own_direction() {
        let back = spin(-8.0);
        assert!((back.period - 45.0).abs() < 1e-9);
        assert!(back.amp < 0.0);
        assert!((spin(12.0).amp - std::f64::consts::TAU).abs() < 1e-9);
    }

    #[test]
    fn the_newest_level_shows_at_the_top_and_older_ones_further_round() {
        let hud = Hud {
            k: 1.0,
            centre: Point::new(0.0, 0.0),
            shown_at: 0.0,
            rest_at: None,
            final_at: None,
            level: 0.0,
            wave: Arc::from([0.0, 0.0, 1.0, 0.5]),
            newest_at: 1.0,
            rate: 10.0,
        };
        assert_eq!(hud.level_at(1.0, 0.0), 0.5);
        assert_eq!(hud.level_at(1.0, 0.1), 1.0);
        assert_eq!(hud.level_at(1.0, 5.0), 0.0);
    }

    #[test]
    fn the_surface_margin_leaves_room_for_the_bloom_round_the_hud() {
        let cfg = OsdConfig {
            theme: OsdTheme::Jarvis,
            width_px: 600,
            ..OsdConfig::default()
        };
        let theme = PlasmaTheme::default();
        let fonts = load_fonts(&theme).unwrap();
        let look = Look::new(&cfg, &theme, String::new(), &fonts);
        let popup = Popup::new(look, &fonts);
        let reach = popup.view(&vo::Tokens::default()).effect_reach(0.0);
        // The frame's glow comes within a few pixels of the surface edge,
        // and the HUD's bloom spreads it about 18 more.
        let bloom = vo::scene::reach::bloom_reach(6.0);
        for side in [reach.left, reach.top, reach.right, reach.bottom] {
            assert!(side >= bloom, "{reach:?}");
        }
    }
}
