//! The `synthwave` look of the vo popup: a dark card inside a magenta to
//! cyan neon tube, a chrome italic title that names the state, a neon bar
//! waveform over a perspective grid, and bright white centred words.
//!
//! The tube draws itself on from the bottom middle when the popup opens and
//! then flickers on like a real neon tube; when a dictation is cancelled it
//! powers down from the right. The title's letters track in from wide
//! spacing whenever the state changes. The grid scrolls toward the viewer
//! while the recording runs and coasts to a stop after it. Once the final
//! text lands, a ring and a check draw on where the waveform was.
//!
//! The tube, card, grid and waveform are canvases, pure in the frame time,
//! so they move without view updates.
//!
//! While editing ([`SynthwaveSite`]) the rows hold the edit window's lines,
//! still bottom-anchored, and a line keeps its left edge while you type
//! (the sticky centring of `EditView::centre`).
//!
//! Round 2 (R2-04), each an opt-in motion feature:
//!
//! - `liquid-echo`: a water line under the newest line holds its faint
//!   mirror image, which wobbles a little, and each new word sends one flat
//!   ring out from under it, gone in 1 s. Reduced motion: a still mirror
//!   and no rings.
//! - `prism-focus`: the waveform runs into a small glass prism near the
//!   right end and leaves it as five spectrum ribbons, longer and brighter
//!   with the voice. Reduced motion: a still fan whose brightness follows
//!   the voice.
//! - `typewriter-spotlight`: the words are set in a typewriter face; a new
//!   word lands 2 px low at 85 % ink and settles in 120 ms, and a soft cone
//!   of light from above glides to it. Reduced motion: no dip and a still
//!   cone.
//!
//! The mirror and the rings are drawn on a canvas: the EG-09 and EG-10
//! water pool needs `vo-sim`, which the popup does not link.

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{BlendMode, GlyphInstance, GlyphXform, Paint, ShapeInstance, NO_WORD};
use vo::scene::emitter::{WordParticles, WordSource};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::{Glass, GlassKind};
use vo::scene::material::Material;
use vo::scene::path::{icons, make, Contour, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo::scene::taffy::JustifyContent;
use vo::scene::text::PlacedGlyph;
use vo::scene::text::TextLayout;
use vo::scene::{Loop, LoopProp};

/// Layout at the design width of 600 logical pixels; everything scales
/// with the popup's width.
const DESIGN_W: f32 = 600.0;
/// Transparent room round the card for the tube's glow.
const MARGIN: f32 = 18.0;
/// Card height above the first transcript line and below the last one.
const HEAD: f32 = 233.0;
const FOOT: f32 = 73.0;
/// Transcript size and line pitch.
const TEXT_PX: f32 = 33.0;
const TEXT_LINE: f32 = 42.0;
/// Transcript width.
const TEXT_W: f32 = 520.0;
/// Title size and centre line.
const TITLE_PX: f32 = 72.0;
const TITLE_Y: f32 = 55.0;
/// Waveform centre line, tallest half height and bar pitch.
const WAVE_Y: f32 = 151.0;
const WAVE_HALF: f32 = 62.0;
const WAVE_X0: f32 = 18.0;
const WAVE_X1: f32 = 582.0;
const BAR_PITCH: f32 = 7.1;
const BAR_W: f32 = 2.8;
/// Bottom of the grid under the waveform.
const GRID_BOTTOM: f32 = 262.0;
/// Status row centre, above the card's bottom edge.
const STATUS_UP: f32 = 40.0;
const STATUS_PX: f32 = 18.5;
/// The tube: stroke width and the corner cuts.
const TUBE_W: f32 = 2.4;
const CUT: f32 = 30.0;

/// Seconds for the tube to draw on, then to flicker on.
const DRAW_SECS: f64 = 0.52;
const FLICKER_SECS: f64 = 0.30;
/// Seconds for the card to follow a new line.
const GROW_SECS: f64 = 0.32;
/// Seconds for the waveform to settle to dots after the stop.
const SETTLE_SECS: f64 = 0.4;
/// Seconds for the grid to coast to a stop, and per grid row while running.
const COAST_SECS: f64 = 0.6;
const GRID_ROW_SECS: f64 = 1.2;
/// With `onset-grid` (MO-67): each onset sends one row from the horizon.
pub(crate) const ONSET_GRID: &str = "onset-grid";
/// Depth a new row starts at (the even rows start there too) and the depth
/// where it leaves at the bottom.
const ROW_FAR: f32 = 7.3;
const ROW_NEAR: f32 = 0.8;
/// Even rows come one step nearer per `GRID_ROW_SECS`.
const ROW_STEP: f32 = 0.7;
/// An onset's row crosses the whole road in this long, so at a speaking
/// rate the road holds the last few seconds of rhythm and a pause clears it.
const ONSET_TRAVEL_SECS: f64 = 2.4;
/// With `light-line` (MO-68): at each onset the tube's light leaves the
/// left edge, runs under the new word, holds and fades. Seconds for the run
/// to the word, the stroke under it, the hold and the fade.
const LIGHT_LEAVE: f64 = 0.18;
const LIGHT_DRAW: f64 = 0.22;
const LIGHT_HOLD: f64 = 0.2;
const LIGHT_FADE: f64 = 0.25;
/// The underline sits this far under the font's descenders, so it never
/// touches a letter.
const LIGHT_GAP: f32 = 3.0;
/// The frame pace while the popup records room tone (RD-11). The grid,
/// the waveform and the tube's hum are canvases that redraw every frame,
/// and at 15 fps they cost 2.3 % of a core; their slow motion reads the
/// same at this pace.
pub(crate) const ROOM_TONE_FPS: f64 = 10.0;
/// Seconds for the tube to power down on a cancel.
const POWER_DOWN_SECS: f64 = 0.45;
/// How far the lit tube's brightness wanders while it hums.
const HUM: f32 = 0.03;
/// Idle motion (RD-02). Once the road has been empty of onset rows for a
/// while, faint rows drift toward the viewer at this speed (logical pixels
/// a second, at the nearest row shown), at this share of a row's light, and
/// the gaps between them breathe by this many pixels on this cycle. They
/// stop above the transcript's first row.
const IDLE_SCROLL_PX: f32 = 8.0;
const IDLE_ROW_ALPHA: f32 = 0.5;
const IDLE_GAP_PX: f32 = 1.0;
const IDLE_GAP_SECS: f64 = 3.0;
const IDLE_ROW_LIFT: f32 = 12.0;
/// Seconds for the idle rows to fade in after the road clears, and out
/// when an onset sends a row.
const IDLE_IN_SECS: f32 = 1.0;
const IDLE_OUT_SECS: f32 = 0.3;
/// Idle motion: the tube dips to this brightness for one step of the room
/// tone pace (0.1 s), once every this many seconds.
const DIP_TO: f64 = 0.7;
const DIP_EVERY: f64 = 8.0;
/// Seconds per cycle of the "..." after "Loading".
const DOTS_SECS: f64 = 1.2;
/// When the title first tracks in, after the tube has started drawing, and
/// how long the old word takes to fade out when the state changes.
const TITLE_DELAY: f64 = 0.22;
const TITLE_SWAP_SECS: f64 = 0.16;
/// A new row's words wait this long while the rows above rise out of its
/// place, so they never land on the row that is leaving (the envelope's
/// pending hold covers it).
const ROW_HOLD_SECS: f64 = 0.16;

/// R2-04-a: the mirror's ink, how far it is squashed, its gap under the
/// descenders and its wobble (pixels, seconds a cycle); the water line's
/// alpha; a ring's life, alpha, reach (quiet and loud) and flatness.
pub(crate) const LIQUID_ECHO: &str = "liquid-echo";
const ECHO_ALPHA: f32 = 0.16;
const ECHO_SQUASH: f32 = 0.42;
const ECHO_GAP: f32 = 2.0;
const ECHO_WOBBLE: f32 = 0.9;
const ECHO_WOBBLE_SECS: f64 = 1.8;
const SURFACE_ALPHA: f32 = 0.14;
const RING_SECS: f64 = 1.0;
const RING_ALPHA: f32 = 0.2;
const RING_REACH: (f32, f32) = (16.0, 40.0);
const RING_FLAT: f32 = 0.2;
/// R2-04-b: the prism's centre and size, where the bars stop short of it,
/// the ribbons' colours, alpha at full voice, length (quiet and loud),
/// half spread in radians, and how long the voice is averaged over.
pub(crate) const PRISM_FOCUS: &str = "prism-focus";
const PRISM_X: f32 = 500.0;
const PRISM_W: f32 = 34.0;
const PRISM_H: f32 = 30.0;
const PRISM_IN: f32 = 478.0;
const FAN: [u32; 5] = [0xff4d6d, 0xffb340, 0x5cf08a, 0x33c8ff, 0xa66bff];
const FAN_ALPHA: f32 = 0.3;
const FAN_LEN: (f32, f32) = (14.0, 76.0);
const FAN_SPREAD: f32 = 0.3;
const FAN_SMOOTH: f64 = 0.25;
/// R2-04-c: a new word's dip and starting ink, the time it takes to
/// settle, and the cone's alpha, height over the word and glide.
pub(crate) const TYPEWRITER_SPOTLIGHT: &str = "typewriter-spotlight";
const DIP_PX: f32 = 2.0;
const DIP_INK: f32 = 0.85;
const DIP_SECS: f64 = 0.12;
const CONE_ALPHA: f32 = 0.12;
const CONE_RISE: f32 = 70.0;
const CONE_GLIDE: f64 = 0.18;
/// Room tone counts as no voice.
const VOICE_GATE: f32 = 0.04;

const MAGENTA: u32 = 0xff2bd6;
const VIOLET: u32 = 0x9b5cff;
const CYAN: u32 = 0x19c8ff;
const BLUE: u32 = 0x3d9bff;
const WHITE: u32 = 0xf6f2ff;

fn hex(rgb: u32) -> Color {
    Color::hex(rgb, 1.0)
}

fn lerp(a: Color, b: Color, k: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

/// The neon hue at `u` across the card, 0 left to 1 right: magenta over
/// the left third, a short violet middle, cyan over the right third.
fn neon(u: f32) -> Color {
    const STOPS: [(f32, u32); 5] = [
        (0.0, MAGENTA),
        (0.36, 0xe030ff),
        (0.5, VIOLET),
        (0.64, 0x4a8cff),
        (1.0, CYAN),
    ];
    let u = u.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let ((u0, c0), (u1, c1)) = (pair[0], pair[1]);
        if u <= u1 {
            return lerp(hex(c0), hex(c1), (u - u0) / (u1 - u0));
        }
    }
    hex(CYAN)
}

/// A fixed pseudo-random value in 0..1 for slot `n`.
fn spike(n: i64) -> f32 {
    let mut x = (n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 32;
    (x & 0xffff) as f32 / 65535.0
}

/// The depth of the row each onset sent (MO-67), seen at `t` with the rows
/// standing as they were at `still`; `run_at` is the road's own clock, which
/// coasts to a stop after the recording. Onsets after `t` have no row yet.
fn onset_depths(onsets: &[f64], t: f64, still: f64, run_at: impl Fn(f64) -> f64) -> Vec<f32> {
    onsets
        .iter()
        .filter(|&&o| o <= t)
        .map(|&o| {
            let age = (run_at(still) - run_at(o)).max(0.0);
            ROW_FAR - (ROW_FAR - ROW_NEAR) * (age / ONSET_TRAVEL_SECS) as f32
        })
        .collect()
}

/// How much of the idle rows show at `t`: none while onset rows cross the
/// road, fading in once it has been empty for a row's whole crossing, and
/// fading out when an onset sends a new row.
fn idle_share(onsets: &[f64], t: f64, shown: f64) -> f32 {
    let (mut last, mut before) = (None, shown - ONSET_TRAVEL_SECS + 0.5);
    for o in onsets.iter().copied().filter(|&o| o <= t) {
        if let Some(l) = last {
            before = l;
        }
        last = Some(o);
    }
    let fade_in = |gap: f64| smoothstep(0.0, IDLE_IN_SECS, (gap - ONSET_TRAVEL_SECS) as f32);
    match last {
        None => fade_in(t - before),
        Some(o) => {
            let was = fade_in(o - before);
            was * (1.0 - smoothstep(0.0, IDLE_OUT_SECS, (t - o) as f32)) + fade_in(t - o)
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// R2-04-b: the voice at `t`, averaged over the last `FAN_SMOOTH` seconds.
fn fan_level(levels: &kit::Levels, t: f64) -> f32 {
    const N: usize = 5;
    (0..N)
        .map(|i| levels.at(t - FAN_SMOOTH * i as f64 / N as f64))
        .map(|v| if v < VOICE_GATE { 0.0 } else { v })
        .sum::<f32>()
        / N as f32
}

/// R2-04-b: how strong the fan is at voice `rms`, 0.25 quiet to 1 loud.
fn fan_power(rms: f32) -> f32 {
    0.25 + 0.75 * (rms * 2.2).clamp(0.0, 1.0)
}

/// R2-04-b: a ribbon's length at voice `rms`, in design pixels.
fn fan_len(rms: f32) -> f32 {
    let k = (rms * 2.2).clamp(0.0, 1.0);
    FAN_LEN.0 + (FAN_LEN.1 - FAN_LEN.0) * k
}

/// R2-04-a: the rings alive at `t`, one per word born in the last
/// `RING_SECS`, as (age in seconds, word's index).
fn echo_rings(borns: &[f64], t: f64) -> Vec<(f64, usize)> {
    borns
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (0.0..RING_SECS).contains(&(t - b)).then_some((t - b, i)))
        .collect()
}

/// R2-04-a: a ring's radius (design pixels) and alpha at `age`, from a
/// word heard at voice `level`.
fn echo_ring(age: f64, level: f32) -> (f32, f32) {
    let k = (age / RING_SECS).clamp(0.0, 1.0) as f32;
    let reach = RING_REACH.0 + (RING_REACH.1 - RING_REACH.0) * (level * 2.2).clamp(0.0, 1.0);
    let grow = 1.0 - (1.0 - k).powi(2);
    (reach * grow, RING_ALPHA * (1.0 - k).powf(1.5))
}

/// R2-04-c: a new word's entry: 2 px low at 85 % ink, settling on the
/// words' 120 ms springs. Reduced motion: no dip.
fn dip_enter(still: bool) -> Transition {
    let enter = Transition::fade().opacity(DIP_INK);
    if still {
        enter
    } else {
        enter.offset(0.0, DIP_PX)
    }
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

/// The look's own fonts, added to each registry once.
#[derive(Clone, Copy)]
struct Fonts {
    title: u32,
    text: u32,
    status: u32,
}

fn fonts(reg: &FontRegistry) -> Fonts {
    // Added to the registry once (`FontRegistry::system`), so every
    // registry, such as a test's, has its own ids.
    let add = |pattern: &str, fallback: u32| reg.system(pattern).unwrap_or(fallback);
    Fonts {
        title: add("Exo 2:black:italic", SANS),
        text: add("Roboto:medium", SANS),
        status: add("Roboto", SANS),
    }
}

/// The transcript's style, for the line fitting in `Popup::text` too.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    let s = look.width / DESIGN_W;
    let mut font = fonts(&look.fonts).text;
    // R2-04-c: the typewriter face.
    if look.on(TYPEWRITER_SPOTLIGHT) {
        font = look.fonts.asset("typewriter").unwrap_or(font);
    }
    TextStyle::new(font, TEXT_PX * s, TEXT_LINE * s)
}

/// The centre of the newest transcript line, from the surface top. The card
/// grows upward as lines are added, so the newest line stays at this spot.
pub(crate) fn newest_line_centre(look: &Look) -> f32 {
    let s = look.width / DESIGN_W;
    let tall = (HEAD + FOOT + swiss::MAX_LINES as f32 * TEXT_LINE) * s;
    MARGIN + tall - (FOOT + TEXT_LINE / 2.0) * s
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * look.width / DESIGN_W
}

/// What only the synthwave look keeps: when the card last changed height,
/// the title's word and when it last changed, and the newest row's first
/// word and when a new newest row came.
#[derive(Default)]
pub struct State {
    lines_from: u32,
    lines_to: u32,
    lines_at: f64,
    title: &'static str,
    title_at: f64,
    row_key: Option<u32>,
    row_at: f64,
    /// A hash of the words the rows were last split from, so room tone's
    /// messages do not lay the text out again.
    rows_of: u64,
}

/// The newest transcript row, in surface pixels: its left edge and top,
/// its words' boxes and glyphs from there, the font's descent under the
/// baseline and the text size.
struct NewestRow {
    x: f32,
    top: f32,
    boxes: Vec<sounding::WordBox>,
    glyphs: Vec<PlacedGlyph>,
    descent: f32,
    size: f32,
}

/// The card outline as two halves from the bottom middle, left and right
/// round to the top middle, so it draws on both ways at once. `w` and `h`
/// are the card's size; the stroke sits `inset` inside it.
fn tube_halves(w: f32, h: f32, s: f32, inset: f32) -> [Contour; 2] {
    let left = tube_points(w, h, s, inset);
    let right: Vec<Point> = left.iter().map(|p| Point::new(w - p.x, p.y)).collect();
    [
        Contour::polyline(&left, false),
        Contour::polyline(&right, false),
    ]
}

/// The left half of the outline, from the bottom middle to the top middle.
fn tube_points(w: f32, h: f32, s: f32, inset: f32) -> Vec<Point> {
    let (c, i) = (CUT * s, inset);
    let low = 38.0 * s; // bottom corner cut, up the side
    let tab = 11.0 * s; // the lower middle tab's depth
    let foot = 30.0 * s; // the bottom corner cut, along the bottom
    let tab_x = 120.0 * s;
    vec![
        Point::new(w / 2.0, h - i),
        Point::new(tab_x + tab, h - i),
        Point::new(tab_x, h - tab - i),
        Point::new(foot, h - tab - i),
        Point::new(i, h - low),
        Point::new(i, c),
        Point::new(c, i),
        Point::new(w / 2.0, i),
    ]
}

/// Short lines inside the four corners, parallel to the cuts.
fn corner_ticks(w: f32, h: f32, s: f32) -> Vec<Contour> {
    let (c, d) = (CUT * s, 9.0 * s);
    let low = 38.0 * s;
    let foot = 30.0 * s;
    let tab = 11.0 * s;
    let left = vec![
        // Top left: along the cut, with a short drop at its lower end.
        vec![
            Point::new(d, c + d + 10.0 * s),
            Point::new(d, c + 4.0 * s),
            Point::new(c + 4.0 * s, d),
            Point::new(c + 26.0 * s, d),
        ],
        // Bottom left: along the cut and the bottom.
        vec![
            Point::new(d, h - low - 16.0 * s),
            Point::new(d, h - low + 4.0 * s),
            Point::new(foot + 6.0 * s, h - tab - d),
            Point::new(foot + 48.0 * s, h - tab - d),
        ],
    ];
    let mut out = Vec::new();
    for pts in left {
        out.push(Contour::polyline(&pts, false));
        let mirrored: Vec<Point> = pts.iter().map(|p| Point::new(w - p.x, p.y)).collect();
        out.push(Contour::polyline(&mirrored, false));
    }
    out
}

/// The card's fill shape.
fn card_shape(w: f32, h: f32, s: f32) -> Contour {
    card_shape_inset(w, h, s, 0.0)
}

/// The card's outline `inset` inside its edge, as one closed contour.
fn card_shape_inset(w: f32, h: f32, s: f32, inset: f32) -> Contour {
    // Bottom middle up the left to the top middle, then down the right.
    let left = tube_points(w, h, s, inset);
    let mut pts = left[..left.len() - 1].to_vec();
    pts.extend(left.iter().rev().map(|p| Point::new(w - p.x, p.y)));
    // `polyline` draws no closing segment, so the last point is the first.
    Contour::polyline(&pts, true)
}

/// Brightness of the tube `k` seconds after it started to flicker on.
fn flicker_on(k: f64) -> f32 {
    const STEPS: [f32; 5] = [1.0, 0.2, 0.9, 0.35, 1.0];
    if k < 0.0 {
        return 1.0;
    }
    let i = (k / (FLICKER_SECS / STEPS.len() as f64)) as usize;
    STEPS.get(i).copied().unwrap_or(1.0)
}

impl Popup {
    /// Brings the look's own state up to date after a message.
    pub(crate) fn sync_synthwave(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Synthwave {
            return;
        }
        let title = self.title_word();
        let st = &mut self.synth;
        if title != st.title {
            if !st.title.is_empty() {
                st.title_at = t;
            }
            st.title = title;
        }
        let words = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.look.width.to_bits().hash(&mut h);
            for w in &self.shown {
                (w.id, &w.text).hash(&mut h);
            }
            h.finish()
        };
        if self.edit_view.is_none() && words != self.synth.rows_of {
            let key = self.sw_rows().last().and_then(|r| r.first()).map(|w| w.id);
            let st = &mut self.synth;
            st.rows_of = words;
            if key.is_some() && key != st.row_key {
                if st.row_key.is_some() {
                    st.row_at = t;
                }
                st.row_key = key;
            }
        }
        let st = &mut self.synth;
        if st.lines_to == 0 {
            st.lines_from = self.lines;
            st.lines_to = self.lines;
        } else if self.lines != st.lines_to {
            st.lines_from = st.lines_to;
            st.lines_to = self.lines;
            st.lines_at = t;
        }
    }

    /// Wakes the view once the old title has left, so the new one tracks
    /// in, and once a new row's place is clear, so its words show.
    pub(crate) fn arm_synthwave_tick(&mut self, cx: &mut Cx<Msg>) {
        let now = cx.t();
        let back = [
            self.synth.title_at + TITLE_SWAP_SECS,
            self.synth.row_at + ROW_HOLD_SECS,
        ]
        .into_iter()
        .filter(|&b| b > now)
        .reduce(f64::min);
        if let Some(back) = back {
            if self.look.theme == OsdTheme::Synthwave && self.extra.tick.is_none() {
                self.extra.tick = Some(cx.after(back - now + 0.01, Msg::Tick));
            }
        }
    }

    fn scale(&self) -> f32 {
        self.look.width / DESIGN_W
    }

    fn card_h(&self, lines: u32) -> f32 {
        (HEAD + FOOT + lines as f32 * TEXT_LINE) * self.scale()
    }

    pub(crate) fn synthwave_surface(look: &Look) -> Size {
        let s = look.width / DESIGN_W;
        let h = (HEAD + FOOT + swiss::MAX_LINES as f32 * TEXT_LINE) * s;
        Size::new(
            (look.width + 2.0 * MARGIN).ceil(),
            (h + 2.0 * MARGIN).ceil(),
        )
    }

    fn now(&self) -> f64 {
        self.extra.now
    }

    /// When the dictation ended without text: cancelled, no speech heard,
    /// or failed. The host keeps the popup up a moment for these.
    fn cancel_at(&self) -> Option<f64> {
        match self.ending()? {
            Ending::Cancelled | Ending::Empty | Ending::Failed => self.ended_at,
            Ending::Done | Ending::Stopped => None,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel_at().is_some()
    }

    /// The word the title shows.
    fn title_word(&self) -> &'static str {
        let status = self.status();
        match self.ending() {
            Some(Ending::Cancelled) => return "CANCELLED",
            Some(Ending::Empty) => return "NO SPEECH",
            Some(Ending::Failed) => return "FAILED",
            Some(Ending::Stopped) => return "STOPPED",
            Some(Ending::Done) | None => {}
        }
        if self.extra.final_at.is_some() {
            self.delivery_word_caps()
        } else if self.ending().is_some() {
            "DONE"
        } else if status.loading {
            "LOADING"
        } else if status.finishing || self.extra.finish_at.is_some() {
            "FINISHING"
        } else {
            "LISTENING"
        }
    }

    pub(crate) fn synthwave_view(&self) -> Element<Msg> {
        let l = &self.look;
        let surface = Self::synthwave_surface(l);
        let root = Element::column("root").size(surface.w, surface.h);
        if self.closing {
            return root;
        }
        let w = l.width;
        let h = self.card_h(self.lines);
        let card = Element::layer("card")
            .size(w, h)
            .child(at(self.sw_grid_and_wave(), 0.0, 0.0))
            .children(self.sw_prism())
            .child(self.sw_done_mark())
            .child(self.sw_title())
            .child(self.sw_status(h));
        // The card's bottom stays put and it grows upward: its content's top
        // springs up as lines are added, on the same spring as the height
        // the card and tube canvases draw. The transcript hangs from the
        // card's bottom instead, so the newest line stays put (RD-09).
        let tall = self.card_h(swiss::MAX_LINES as u32);
        let s = self.scale();
        let text_top = MARGIN + tall - (FOOT + swiss::MAX_LINES as f32 * TEXT_LINE) * s;
        root.child(at(self.sw_card(), 0.0, 0.0))
            .child(
                at(card, MARGIN, MARGIN + tall - h)
                    .motion(Prop::Position, Spring::critical_to_98(GROW_SECS)),
            )
            .children(self.sw_spotlight(text_top))
            .child(at(self.sw_text(), MARGIN, text_top))
            .children(self.sw_echo(text_top))
            .children(self.sw_light_line(text_top))
            .child(at(self.sw_tube(), 0.0, 0.0))
    }

    /// The card's height at time `t`: it springs to each new line count.
    fn height_fn(&self) -> impl Fn(f64) -> f32 + Send + Sync + 'static {
        let st = &self.synth;
        let (from, to) = (self.card_h(st.lines_from), self.card_h(st.lines_to.max(1)));
        let since = st.lines_at;
        let spring = Spring::critical_to_98(GROW_SECS);
        move |t| {
            if from == to || t <= since {
                return if t <= since { from } else { to };
            }
            to + spring.eval((from - to) as f64, 0.0, t - since).0 as f32
        }
    }

    fn sw_until(&self) -> f64 {
        let mut until = self.shown_at + DRAW_SECS + FLICKER_SECS + 0.1;
        until = until.max(self.synth.lines_at + GROW_SECS + 0.1);
        if let Some(c) = self.cancel_at() {
            until = until.max(c + POWER_DOWN_SECS + 0.1);
        }
        if let Some(f) = self.extra.final_at {
            until = until.max(f + 0.6);
        }
        until
    }

    /// The dark card behind everything.
    fn sw_card(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let height = self.height_fn();
        let shown = self.shown_at;
        let tall = self.card_h(swiss::MAX_LINES as u32);
        let surface = Self::synthwave_surface(&self.look);
        Element::canvas("card-fill", self.sw_until(), move |t, rect, p| {
            let h = height(t);
            let open = smoothstep(0.0, 0.25, (t - shown) as f32);
            // The card's bottom stays put; its top rises with its height.
            let (ox, oy) = (rect.x + MARGIN, rect.y + MARGIN + tall - h);
            let shape = card_shape(w, h, s).transform(&vo::scene::geom::Affine::translate(ox, oy));
            let style = StrokeStyle::solid(0.0, Color::hex(0x0d0828, 0.9 * open)).linear(
                Point::new(ox, oy),
                Color::hex(0x0a0722, 0.92 * open),
                Point::new(ox, oy + h),
                Color::hex(0x050314, 0.94 * open),
            );
            p.fill([&shape], &style);
        })
        .size(surface.w, surface.h)
    }

    /// The neon tube and the corner ticks, with the glow and bloom.
    fn sw_tube(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let height = self.height_fn();
        let shown = self.shown_at;
        let cancel = self.cancel_at();
        let pulse = self.extra.final_at;
        let surface = Self::synthwave_surface(&self.look);
        let tall = self.card_h(swiss::MAX_LINES as u32);
        let draw = Element::canvas("tube", self.sw_until(), move |t, rect, p| {
            let h = height(t);
            let (ox, oy) = (rect.x + MARGIN, rect.y + MARGIN + tall - h);
            let move_to = vo::scene::geom::Affine::translate(ox, oy);
            let k = ((t - shown) / DRAW_SECS).clamp(0.0, 1.0);
            let drawn = 1.0 - (1.0 - k as f32).powi(3);
            let mut bright = flicker_on(t - shown - DRAW_SECS);
            if let Some(f) = pulse {
                let a = ((t - f) / 0.4).clamp(0.0, 1.0) as f32;
                if t >= f {
                    bright *= 1.0 + 0.5 * (1.0 - a);
                }
            }
            // A cancel powers the tube down from the right.
            let down = cancel.map_or(0.0, |c| ((t - c) / POWER_DOWN_SECS).clamp(0.0, 1.0) as f32);
            let gradient = |a: f32, width: f32, glow: f32| {
                StrokeStyle::solid(width, hex(MAGENTA))
                    .linear(
                        Point::new(ox, oy),
                        hex(MAGENTA).fade(a),
                        Point::new(ox + w, oy),
                        hex(CYAN).fade(a * (1.0 - 0.8 * down)),
                    )
                    .glow(glow)
            };
            let fade = 1.0 - 0.75 * down;
            let width = TUBE_W * s.max(1.0);
            if k >= 1.0 {
                // Drawn on: one closed line, so no caps meet at the seams.
                let ring = card_shape_inset(w, h, s, TUBE_W / 2.0).transform(&move_to);
                p.stroke(&ring, &gradient(bright * fade, width, 6.0));
                p.stroke(
                    &ring,
                    &StrokeStyle::solid(0.9, Color::hex(0xffffff, 0.55 * bright * fade)),
                );
            }
            for half in tube_halves(w, h, s, TUBE_W / 2.0).into_iter().filter(|_| k < 1.0) {
                let half = half.transform(&move_to);
                let len = half.length() * drawn;
                let piece = half.trim(0.0, len);
                p.stroke(&piece, &gradient(bright * fade, width, 6.0));
                // A white hot core along the tube.
                p.stroke(
                    &piece,
                    &StrokeStyle::solid(0.9, Color::hex(0xffffff, 0.55 * bright * fade)),
                );
                // The hot tip while drawing on.
                if k < 1.0 {
                    if let Some((tip, _)) = half.at(len) {
                        p.shape(ShapeInstance::circle(
                            tip,
                            2.6,
                            Paint::solid(Color::hex(0xffffff, 0.95)),
                        ));
                    }
                }
            }
            // The corner ticks light once the tube has passed them.
            let ticks = smoothstep(0.6, 1.0, drawn) * bright * fade;
            for c in corner_ticks(w, h, s) {
                let c = c.transform(&move_to);
                let len = c.length();
                p.stroke(&c.trim(0.0, len * ticks.min(1.0)), &gradient(0.75 * ticks, 1.1, 2.0));
            }
        })
        .size(surface.w, surface.h);
        // Idle motion: now and then the tube dips, as a real one does.
        let lit = self.now() >= self.shown_at + DRAW_SECS + FLICKER_SECS;
        let dips = lit
            && !self.cancelled()
            && self.extra.finish_at.is_none()
            && !self.look.on("reduced-motion");
        let draw = if dips {
            let step = 1.0 / ROOM_TONE_FPS / DIP_EVERY;
            let gate = Loop::gate(0.5 - step / 2.0, 0.5 + step / 2.0, DIP_EVERY);
            let mut dip = gate.fps(ROOM_TONE_FPS);
            dip.amp = DIP_TO - 1.0;
            draw.ambient(LoopProp::Opacity, dip)
        } else {
            draw
        };
        let layer = Element::layer("tube-light")
            .size(surface.w, surface.h)
            .blend(BlendMode::Add)
            .bloom(0.9, 8.0)
            .opacity(1.0 - HUM)
            .child(draw);
        // Once the tube is on and until a cancel, it hums: its brightness and
        // its bloom wander a little, as a real neon tube's do.
        if lit && !self.cancelled() {
            layer
                .ambient(LoopProp::Opacity, Loop::drift(HUM as f64, 0.11, 7))
                .ambient(LoopProp::BloomStrength, Loop::drift(0.12, 0.3, 11))
        } else {
            layer
        }
    }

    /// The perspective grid and the bar waveform, in one bloom layer.
    fn sw_grid_and_wave(&self) -> Element<Msg> {
        let l = &self.look;
        let (w, s) = (l.width, self.scale());
        let shown = self.shown_at;
        let status = self.status();
        let stop = self.extra.finish_at.or(self.cancel_at());
        let done = self.extra.final_at;
        let loading = status.loading;
        let fraction = self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32);
        let finishing = self.extra.finish_at.is_some() && done.is_none();
        let horizon = WAVE_Y * s;
        let bottom = GRID_BOTTOM * s;
        let grid_until = stop.map_or(f64::INFINITY, |f| f + COAST_SECS + 0.05);
        // MO-67: the rows are the onsets, so the road's spacing is the
        // speech rhythm and silence is empty road. Under reduced motion the
        // rows stand still between onsets.
        let onset_rows = l.on(ONSET_GRID);
        let reduced = l.on("reduced-motion");
        let onsets: std::sync::Arc<[f64]> = if onset_rows {
            self.heard.onsets().into()
        } else {
            std::sync::Arc::from([])
        };
        let idle_rows = onset_rows && !reduced;
        let row_limit = (HEAD - IDLE_ROW_LIFT) * s;
        let grid = Element::canvas("grid", grid_until, move |t, rect, p| {
            // Rows scroll toward the viewer and coast to a stop after the stop.
            let run_at = |t: f64| {
                match stop {
                    Some(f) if t > f => {
                        let k = ((t - f) / COAST_SECS).min(1.0);
                        (f - shown) + COAST_SECS * (k - 0.5 * k * k)
                    }
                    _ => t - shown,
                }
                .max(0.0)
            };
            let run = run_at(t);
            let phase = (run / GRID_ROW_SECS).fract() as f32;
            let open = smoothstep(0.1, 0.5, (t - shown) as f32);
            let (x0, y0) = (rect.x, rect.y + horizon);
            let span = bottom - horizon;
            let cx = x0 + w / 2.0;
            // Rays from a vanishing point a little above the horizon.
            let vy = y0 - 10.0 * s;
            for k in -10..=10 {
                let bx = cx + k as f32 * 64.0 * s;
                let along = |y: f32| cx + (bx - cx) * (y - vy) / (bottom + rect.y - vy);
                let (a, b) = (Point::new(along(y0), y0), Point::new(along(y0 + span), y0 + span));
                let hue = neon((bx - x0) / w);
                let style = StrokeStyle::solid(1.0, hue).linear(
                    a,
                    hue.fade(0.55 * open),
                    b,
                    hue.fade(0.0),
                );
                p.stroke(&Contour::polyline(&[a, b], false), &style);
            }
            // Rows at depth z land at y = horizon + span / z.
            let still = if reduced {
                onsets.iter().copied().filter(|&o| o <= t).fold(shown, f64::max)
            } else {
                t
            };
            let depths: Vec<f32> = if onset_rows {
                onset_depths(&onsets, t, still, run_at)
            } else {
                (0..10).map(|n| 1.0 + (n as f32 - phase) * ROW_STEP).collect()
            };
            // Idle motion: faint rows drift in once the road is empty.
            let quiet = if idle_rows {
                idle_share(&onsets, t, shown)
            } else {
                0.0
            };
            if quiet > 0.0 {
                let reach = span * 0.92;
                let near = reach / (row_limit - horizon);
                let rate = IDLE_SCROLL_PX * s * near * near / (reach * ROW_STEP);
                let breathe = (std::f64::consts::TAU * t / IDLE_GAP_SECS).sin() as f32;
                let step = ROW_STEP + 0.5 * IDLE_GAP_PX * s * near * near / reach * breathe;
                let phase = (run * rate as f64).fract() as f32;
                for n in 0..12 {
                    let z = near + (n as f32 + 1.0 - phase) * step;
                    let y = y0 + reach / z;
                    let depth = (y - y0) / span;
                    let a = open
                        * 0.55
                        * IDLE_ROW_ALPHA
                        * quiet
                        * (1.0 - depth).powf(0.8)
                        * smoothstep(0.02, 0.12, depth)
                        * smoothstep(row_limit, row_limit - 20.0 * s, y - rect.y);
                    if a <= 0.0 {
                        continue;
                    }
                    let style = StrokeStyle::solid(1.0, hex(VIOLET)).linear(
                        Point::new(x0, y),
                        hex(MAGENTA).fade(a),
                        Point::new(x0 + w, y),
                        hex(CYAN).fade(a),
                    );
                    p.stroke(
                        &Contour::polyline(&[Point::new(x0, y), Point::new(x0 + w, y)], false),
                        &style,
                    );
                }
            }
            for z in depths {
                if z < ROW_NEAR {
                    continue;
                }
                let y = y0 + span / z * 0.92;
                if y > y0 + span {
                    continue;
                }
                let depth = (y - y0) / span;
                let a = open * 0.55 * (1.0 - depth).powf(0.8) * smoothstep(0.02, 0.12, depth);
                let style = StrokeStyle::solid(1.0, hex(VIOLET)).linear(
                    Point::new(x0, y),
                    hex(MAGENTA).fade(a),
                    Point::new(x0 + w, y),
                    hex(CYAN).fade(a),
                );
                p.stroke(
                    &Contour::polyline(&[Point::new(x0, y), Point::new(x0 + w, y)], false),
                    &style,
                );
            }
        })
        .size(w, bottom + 4.0);
        let grid = Element::layer("grid-fade")
            .size(w, bottom + 4.0)
            .fade_edges(110.0 * s, 0.0, 110.0 * s, 40.0 * s)
            .child(grid);

        let levels = self.wave.clone();
        let newest_at = self.newest_at;
        let window = l.window;
        // R2-04-b: the bars run into the prism, and the fan leaves it.
        let prism = l.on(PRISM_FOCUS);
        let fan_levels = self.kit_levels();
        let still = self.still();
        let wx1 = if prism { PRISM_IN } else { WAVE_X1 };
        let (wx0, wx1) = (WAVE_X0 * s, wx1 * s);
        let half_max = WAVE_HALF * s;
        let pitch = BAR_PITCH * s;
        let bar_w = BAR_W * s;
        let wave_until = match (stop, done) {
            (_, Some(d)) => d + 0.4,
            (Some(f), None) if !finishing => f + SETTLE_SECS + 0.05,
            (Some(_), None) => f64::INFINITY,
            (None, None) => f64::INFINITY,
        };
        let wave = Element::canvas("wave", wave_until, move |t, rect, p| {
            let open = smoothstep(0.15, 0.5, (t - shown) as f32);
            let settle = stop.map_or(1.0, |f| {
                1.0 - smoothstep(0.0, 1.0, ((t - f) / SETTLE_SECS) as f32)
            });
            let gone = done.map_or(1.0, |d| 1.0 - smoothstep(0.0, 1.0, ((t - d) / 0.3) as f32));
            let cy = rect.y + horizon;
            let n = ((wx1 - wx0) / pitch) as i32;
            let now = t.min(newest_at.max(shown));
            let slot = window / n as f64;
            // Bars sit on a grid of slot times, so a bar keeps its height while it scrolls.
            let last = (now / slot).floor() as i64;
            let shift = ((now / slot).fract()) as f32 * pitch;
            for j in 0..=n {
                let x = rect.x + wx1 - j as f32 * pitch - shift;
                if x < rect.x + wx0 {
                    break;
                }
                let u = (x - rect.x - wx0) / (wx1 - wx0);
                let slot_t = (last - j as i64) as f64 * slot;
                let age = newest_at - slot_t;
                let i = levels.len() as f64 - 1.0 - age * FRAME_HZ as f64;
                let level = if loading || i < 0.0 || levels.is_empty() {
                    0.0
                } else {
                    levels[(i as usize).min(levels.len() - 1)]
                };
                // More contrast than the raw level, and a fixed spike per slot, for
                // the spiky look of the mock-up.
                let level = (level / 0.7).clamp(0.0, 1.0).powf(0.9)
                    * (0.45 + 0.55 * spike(last - j as i64));
                let env = (std::f32::consts::PI * u).sin().powf(0.9) * smoothstep(0.03, 0.2, u)
                    * smoothstep(0.97, 0.8, u);
                let half = (1.3 * s + level.min(1.2) * half_max * env * open) * settle
                    + 1.3 * s * (1.0 - settle);
                let mut hue = neon(u);
                let mut alpha = open * gone;
                // Loading: a bright run moves along the dotted line.
                if loading {
                    let head = fraction.unwrap_or(((t - shown) * 0.5).fract() as f32);
                    let d = (u - head).abs();
                    alpha *= 0.45 + 0.55 * (1.0 - smoothstep(0.0, 0.05, d));
                }
                // Finishing: grey-violet right of the playhead.
                if finishing {
                    let head = fraction.unwrap_or(0.0);
                    if u > head {
                        hue = lerp(hue, Color::hex(0x5a4a8a, 1.0), 0.75);
                        alpha *= 0.7;
                    }
                }
                let r = RRect::new(
                    Rect::new(x - bar_w / 2.0, cy - half, bar_w, 2.0 * half),
                    bar_w / 2.0,
                );
                p.shape(ShapeInstance::rrect(r, Paint::solid(hue.fade(alpha))));
                // A paler core makes the bar read as a lit tube.
                let core = bar_w * 0.4;
                let rc = RRect::new(
                    Rect::new(x - core / 2.0, cy - half + 0.5, core, (2.0 * half - 1.0).max(0.0)),
                    core / 2.0,
                );
                p.shape(ShapeInstance::rrect(
                    rc,
                    Paint::solid(lerp(hue, Color::WHITE, 0.35).fade(alpha * 0.35)),
                ));
            }
            if finishing {
                if let Some(head) = fraction {
                    let x = rect.x + wx0 + head * (wx1 - wx0);
                    let r = RRect::new(Rect::new(x - 1.2, cy - 16.0 * s, 2.4, 32.0 * s), 1.2);
                    p.shape(ShapeInstance::rrect(
                        r,
                        Paint::solid(hex(CYAN).fade(gone)),
                    ));
                }
            }
            if prism {
                let rms = if loading {
                    0.0
                } else {
                    fan_level(&fan_levels, t)
                };
                let lit = open * settle * gone * FAN_ALPHA * fan_power(rms);
                let len = if still {
                    0.5 * (FAN_LEN.0 + FAN_LEN.1)
                } else {
                    fan_len(rms)
                } * s;
                let from = Point::new(rect.x + (PRISM_X + 0.3 * PRISM_W) * s, cy);
                for (i, &c) in FAN.iter().enumerate() {
                    let turn = FAN_SPREAD * (i as f32 / 2.0 - 1.0);
                    let (dy, dx) = turn.sin_cos();
                    // A gentle travelling wave along the ribbon, none at
                    // the prism and none under reduced motion.
                    let pts: Vec<Point> = (0..=16)
                        .map(|j| {
                            let d = len * j as f32 / 16.0;
                            let wave = if still {
                                0.0
                            } else {
                                1.4 * s
                                    * (j as f32 / 16.0)
                                    * (0.12 * d / s - 5.0 * t as f32 + i as f32).sin()
                            };
                            Point::new(from.x + dx * d - dy * wave, from.y + dy * d + dx * wave)
                        })
                        .collect();
                    let end = pts[16];
                    let style = StrokeStyle::solid(2.2 * s, hex(c))
                        .linear(from, hex(c).fade(lit), end, hex(c).fade(0.0))
                        .glow(3.0 * s);
                    p.stroke(&Contour::polyline(&pts, false), &style);
                }
            }
        })
        .size(w, bottom);
        Element::layer("lights")
            .size(w, bottom + 4.0)
            .blend(BlendMode::Add)
            .bloom(0.45, 6.0)
            .child(at(grid, 0.0, 0.0))
            .child(at(wave, 0.0, 0.0))
    }

    /// Once the final text lands: a ring and a check draw on where the
    /// waveform was.
    fn sw_done_mark(&self) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        let s = self.scale();
        let side = 84.0 * s;
        let x = (self.look.width - side) / 2.0;
        let y = WAVE_Y * s - side / 2.0;
        let layer = Element::layer("done-light")
            .size(side, side)
            .blend(BlendMode::Add)
            .bloom(0.8, 6.0);
        if self.extra.final_at.is_none() || self.cancelled() {
            return at(layer, x, y);
        }
        let ring = RING.get_or_init(|| {
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(32.0, 32.0), 29.0)],
            })
        });
        let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
        let style = |w: f32| {
            StrokeStyle::solid(w, hex(MAGENTA))
                .linear(
                    Point::new(0.0, 0.0),
                    hex(MAGENTA),
                    Point::new(side, side),
                    hex(CYAN),
                )
                .glow(4.0)
        };
        let inner = side * 0.5;
        at(
            layer
                .child(at(
                    Element::stroke_path("ring", ring.clone(), Rect::new(0.0, 0.0, 64.0, 64.0), style(2.6))
                        .size(side, side)
                        .motion(Prop::Trim, Spring::critical_to_98(0.3))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    0.0,
                ))
                .child(at(
                    Element::stroke_path(
                        "check",
                        check.clone(),
                        Rect::new(0.0, 0.0, 24.0, 24.0),
                        style(3.4),
                    )
                    .size(inner, inner)
                    .motion(Prop::Trim, Spring::critical_to_98(0.56))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    (side - inner) / 2.0,
                    (side - inner) / 2.0,
                )),
            x,
            y,
        )
    }

    /// The chrome title: letters cut from a chrome gradient, over a magenta
    /// neon copy of themselves, tracking in on every state change.
    fn sw_title(&self) -> Element<Msg> {
        let (w, s) = (self.look.width, self.scale());
        let f = fonts(&self.look.fonts);
        let px = TITLE_PX * s;
        let line = self.look.fonts.line_height(f.title, px).unwrap_or(px * 1.3);
        let style = TextStyle::new(f.title, px, line).tracking(3.0 * s);
        let box_h = line;
        let box_y = TITLE_Y * s - box_h / 2.0;
        // A new word waits until the old one has faded out, so their
        // letters never cross.
        let shown = self.now() >= self.shown_at + TITLE_DELAY
            && self.now() >= self.synth.title_at + TITLE_SWAP_SECS;
        let word = self.title_word();
        let cancelled = self.cancelled();
        let shaped = self.measure.layout(word, &style, None);
        let total = shaped.size.w;
        let x0 = (w - total) / 2.0;
        let mid = (word.len() as f32 - 1.0) / 2.0;
        let letters = |layer: &'static str| {
            let mut out = Vec::new();
            if !shown {
                return out;
            }
            for (i, ch) in word.chars().enumerate() {
                let Some(g) = shaped.glyphs.iter().find(|g| g.cluster as usize == i) else {
                    continue;
                };
                let d = (i as f32 - mid) / mid.max(1.0);
                let spread = d * 0.6 * total / 2.0;
                let secs = 0.28 + 0.14 * d.abs() as f64;
                out.push(at(
                    Element::label(format!("{layer}-{word}-{i}"), ch.to_string())
                        .text_style(style)
                        .color(Color::WHITE)
                        .motion(Prop::Offset, Spring::critical_to_98(secs))
                        .motion(Prop::Opacity, Spring::critical_to_98(0.14))
                        .presence(
                            Transition::fade().offset(spread, 0.0),
                            Transition::fade().offset(spread * 0.3, -10.0 * s),
                        ),
                    x0 + g.pos.x,
                    0.0,
                ));
            }
            out
        };
        // Chrome on the glyphs themselves, keyed to the em height so the
        // horizon runs straight across the word: white to lavender above,
        // violet to hot pink below. Cancelled turns it red.
        let chrome_fill = if cancelled {
            Material::chrome(
                hex(0x5a0420),
                hex(0xffd6e0),
                hex(0x3a0610),
                hex(0xff3b6b),
            )
        } else {
            Material::chrome(
                hex(0xffffff),
                hex(0xbfa2ff),
                hex(0x3a0b9e),
                hex(0xff5ee8),
            )
        };
        let chrome = Element::layer("title-chrome")
            .size(w, box_h)
            .material(chrome_fill.horizon_line(0.9, 0.02))
            .children(letters("chrome"));
        let glow_hue = if cancelled { 0xff2b5e } else { MAGENTA };
        let glow = Element::layer("title-glow")
            .size(w, box_h)
            .blend(BlendMode::Add)
            .bloom(0.55, 4.0 * s)
            .children(
                letters("glow")
                    .into_iter()
                    .map(|e| e.color(Color::hex(glow_hue, 0.8))),
            );
        at(
            Element::layer("title")
                .size(w, box_h)
                .child(at(glow, 0.0, 2.0 * s))
                .child(at(chrome, 0.0, 0.0)),
            0.0,
            box_y,
        )
    }

    /// The transcript as centred lines. Each line is its own live text, so
    /// a word that wraps leaves one line and rises into the next.
    fn sw_text(&self) -> Element<Msg> {
        let l = &self.look;
        let s = self.scale();
        let style = text_style(l);
        let width = text_width(l);
        // While editing: the window's lines, each with its sticky centring
        // width.
        let edited: Option<Vec<(Vec<Word>, f32)>> = self
            .edit_view
            .is_some()
            .then(|| self.text_lines(&style, width));
        let lines: Vec<Vec<Word>> = match &edited {
            Some(edited) => edited.iter().map(|(w, _)| w.clone()).collect(),
            None => self.sw_rows(),
        };
        let held = self.now() < self.synth.row_at + ROW_HOLD_SECS;
        let dim = if self.cancelled() { 0.3 } else { 1.0 };
        let typewriter = l.on(TYPEWRITER_SPOTLIGHT);
        // The rows sit on the bottom of the tallest card's text box: a new
        // line comes in on the last row and the older ones rise with the
        // card's top, on its spring.
        let mut column = Element::column("text")
            .width(l.width)
            .height(swiss::MAX_LINES as f32 * style.line_height)
            .justify(JustifyContent::FLEX_END)
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.4));
        let newest = lines.len().saturating_sub(1);
        // MO-69: the words form from particles that leave the waveform.
        let particles = (l.on(showcase::PARTICLE_SOURCE)
            && edited.is_none()
            && !self.still()
            && self.extra.finish_at.is_none())
        .then(|| WordParticles {
            color: Color::hex(0xff6ae6, 0.8),
            size: 0.6,
            density: 0.6,
            from: Some(self.sw_particle_source()),
            ..WordParticles::default()
        });
        for (i, line) in lines.iter().enumerate() {
            // A row is keyed by its first word, so on a cut the rows slide
            // up as they are and the top one leaves upward with them; no
            // row ever fades its old words out over its new ones.
            let key = match line.first() {
                Some(w) => format!("{}", w.id),
                None => format!("{i}"),
            };
            let words = Element::live_text(format!("line-{key}"), line)
                .text_style(style)
                .color(hex(WHITE))
                .text_halo(Color::hex(0x6a5cff, 0.9), 11.0 * s)
                .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                .motion(Prop::Position, Spring::critical_to_98(0.2));
            // R2-04-c: the typewriter's dip settles with no overshoot.
            let words = if typewriter {
                words
                    .motion(Prop::Offset, Spring::critical_to_98(DIP_SECS))
                    .word_enter(self.ghost_enter(dip_enter(self.still())))
            } else {
                words
                    .motion(Prop::Offset, Spring::POP)
                    .word_enter(self.ghost_enter(Transition::fade().offset(0.0, 8.0 * s)))
            };
            // MO-14: the newest word's baseline follows the pitch.
            let words = self.typo_words(words);
            let words = match particles {
                Some(p) => words.word_particles(p),
                None => words,
            };
            let row = Element::row(format!("row-{key}"))
                .width(l.width)
                .height(style.line_height)
                .motion(Prop::Position, Spring::critical_to_98(GROW_SECS));
            // On a cut the top row leaves upward with the rows below it.
            // Any other row that goes, on a re-wrap, goes at once, as does
            // the cut row under reduced motion: a fade in place would show
            // the old layout over the new one. The edit window's rows change
            // in place.
            let row = if edited.is_none() {
                let cut = i == 0 && lines.len() >= swiss::MAX_LINES as usize;
                let leave = if cut && !self.still() {
                    Transition::fade().offset(0.0, -style.line_height)
                } else {
                    Transition::NONE
                };
                row.motion(Prop::Offset, Spring::critical_to_98(GROW_SECS))
                    .presence(Transition::NONE, leave)
            } else {
                row
            };
            // A new row takes its place at once, so the rows above start to
            // rise, but its words wait until that place is clear.
            if i == newest && held && edited.is_none() {
                column = column.child(row);
                continue;
            }
            let mut row = match edited.as_ref().and_then(|e| e.get(i)) {
                // Set by its sticky width rather than centred by the row,
                // so it holds still while you type; as wide as the text box,
                // so typing never wraps it.
                Some((_, lw)) => {
                    row.child(at(words.width(width), ((l.width - lw) / 2.0).max(0.0), 0.0))
                }
                None => row.justify(JustifyContent::CENTER).child(words),
            };
            if i == newest && edited.is_none() {
                // The ghost tail after the newest line, where its centred
                // words end, as far as the text's right edge.
                let joined = line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                let lw = self.measure.layout(&joined, &style, None).size.w;
                let tail = model_signals::TailText {
                    style,
                    width: (width + lw) / 2.0,
                    rows: 1,
                };
                row = self.signal_marks_in(row, line, &tail, ((l.width - lw) / 2.0, 0.0), hex(WHITE));
            }
            column = column.child(row);
        }
        if lines.is_empty() {
            // No line yet: the forming cue where the first one goes, from
            // the middle of the row.
            let tail = model_signals::TailText {
                style,
                width: width / 2.0,
                rows: 1,
            };
            let slot = Element::layer("forming-slot")
                .size(l.width, style.line_height);
            column = column.child(self.signal_marks_in(slot, &[], &tail, (l.width / 2.0, 0.0), hex(WHITE)));
        }
        column
    }

    /// The shown words in centred rows, each from where its first glyph
    /// landed.
    fn sw_rows(&self) -> Vec<Vec<Word>> {
        let style = text_style(&self.look);
        let joined: Vec<&str> = self.shown.iter().map(|w| w.text.as_str()).collect();
        let text = joined.join(" ");
        let width = text_width(&self.look);
        let shaped = self.measure.layout(&text, &style, Some(width));
        let mut lines: Vec<Vec<Word>> = Vec::new();
        let mut offset = 0usize;
        let first_y = shaped.glyphs.first().map_or(0.0, |g| g.pos.y);
        for word in &self.shown {
            let y = shaped
                .glyphs
                .iter()
                .find(|g| g.cluster as usize >= offset)
                .map_or(first_y, |g| g.pos.y);
            let line = ((y - first_y) / style.line_height).round().max(0.0) as usize;
            while lines.len() <= line {
                lines.push(Vec::new());
            }
            lines[line].push(word.clone());
            offset += word.text.len() + 1;
        }
        lines
    }

    /// MO-69: the light the words' particles leave from, running along the
    /// waveform's centre line from left to right, in surface pixels.
    fn sw_particle_source(&self) -> WordSource {
        let s = self.scale();
        let tall = self.card_h(swiss::MAX_LINES as u32);
        let top = MARGIN + tall - self.card_h(self.lines);
        WordSource {
            centre: Point::new(MARGIN + self.look.width / 2.0, top + WAVE_Y * s),
            radius: [(WAVE_X1 - WAVE_X0) * 0.35 * s, 1.0],
            from: -std::f32::consts::PI,
            to: 0.0,
            period: 3.0,
            lag: 0.2,
            spread: 0.3,
        }
    }

    /// The newest row as `sw_text` splits and centres it, in surface
    /// pixels. `text_top` is the text column's top.
    fn sw_newest_row(&self, text_top: f32) -> Option<NewestRow> {
        let l = &self.look;
        let style = text_style(l);
        let width = text_width(l);
        let joined: Vec<&str> = self.shown.iter().map(|w| w.text.as_str()).collect();
        let shaped = self.measure.layout(&joined.join(" "), &style, Some(width));
        let last_y = shaped.glyphs.last()?.pos.y;
        let mut offset = 0usize;
        let mut row = Vec::new();
        for w in &self.shown {
            let y = shaped
                .glyphs
                .iter()
                .find(|g| g.cluster as usize >= offset)
                .map_or(last_y, |g| g.pos.y);
            if (y - last_y).abs() < 1.0 {
                row.push(w.clone());
            }
            offset += w.text.len() + 1;
        }
        let (boxes, _) = sounding::word_boxes(&self.measure, &row, &style, None);
        let row_text: Vec<&str> = row.iter().map(|w| w.text.as_str()).collect();
        let laid = self.measure.layout(&row_text.join(" "), &style, None);
        // The font's descent: its own line height less the baseline it sets
        // in that height.
        let own = self
            .measure
            .fonts()
            .line_height(style.font, style.size)
            .unwrap_or(style.line_height);
        let descent = self
            .measure
            .layout(
                "H",
                &TextStyle {
                    line_height: own,
                    ..style
                },
                None,
            )
            .lines
            .first()
            .map_or(0.25 * style.size, |x| own - x.baseline);
        Some(NewestRow {
            x: MARGIN + (l.width - laid.size.w) / 2.0,
            top: text_top + (swiss::MAX_LINES - 1) as f32 * style.line_height,
            boxes,
            glyphs: laid.glyphs.clone(),
            descent,
            size: style.size,
        })
    }

    /// Whether the round 2 marks round the newest line show: while the
    /// words come in, not while editing or after a cancel.
    fn sw_live_row(&self) -> bool {
        self.edit_view.is_none() && !self.cancelled() && self.extra.final_at.is_none()
    }

    /// R2-04-a: the water line under the newest row, its mirror image and
    /// a ring from under each new word. `text_top` is the text column's top.
    fn sw_echo(&self, text_top: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(LIQUID_ECHO)
            || !self.sw_live_row()
            || self.synth.row_at + ROW_HOLD_SECS > self.now()
        {
            return None;
        }
        let row = self.sw_newest_row(text_top)?;
        let s = self.scale();
        let still = self.still();
        let levels = self.kit_levels();
        // Each word's birth and the voice it was heard at.
        let words: Vec<(f64, f32, f32)> = row
            .boxes
            .iter()
            .filter_map(|b| {
                let born = self.heard.word(b.id)?.born;
                Some((born, row.x + b.rect.x + b.rect.w / 2.0, levels.at(born)))
            })
            .collect();
        let baseline = row.boxes.first()?.baseline;
        let water = row.top + baseline + row.descent + ECHO_GAP * s;
        let (x0, x1) = (
            row.x - 12.0 * s,
            row.x + row.boxes.last().map_or(0.0, |b| b.rect.x + b.rect.w) + 12.0 * s,
        );
        let newest = words.iter().map(|w| w.0).fold(f64::NEG_INFINITY, f64::max);
        let until = match (still, self.extra.finish_at) {
            (true, _) => 0.0,
            (false, Some(f)) => f.max(newest + RING_SECS),
            (false, None) => f64::INFINITY,
        };
        let borns: Vec<f64> = words.iter().map(|w| w.0).collect();
        let surface = Self::synthwave_surface(l);
        let ring_y = water + (RING_REACH.1 * RING_FLAT + 1.0) * s;
        let NewestRow {
            x,
            top,
            glyphs,
            size,
            ..
        } = row;
        let canvas = Element::canvas("liquid-echo", until, move |t, r, p| {
            let mid = (x0 + x1) / 2.0;
            let line = |a: f32, b: f32, ca: Color, cb: Color, p: &mut Painter| {
                let (a, b) = (
                    Point::new(r.x + a, r.y + water),
                    Point::new(r.x + b, r.y + water),
                );
                p.stroke(
                    &Contour::polyline(&[a, b], false),
                    &StrokeStyle::solid(1.0 * s, cb).linear(a, ca, b, cb),
                );
            };
            line(
                x0,
                mid,
                hex(MAGENTA).fade(0.0),
                hex(VIOLET).fade(SURFACE_ALPHA),
                p,
            );
            line(
                mid,
                x1,
                hex(VIOLET).fade(SURFACE_ALPHA),
                hex(CYAN).fade(0.0),
                p,
            );
            // The mirror: the row flipped at the water line and squashed.
            let w = r.y + water;
            let flip = Affine {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -ECHO_SQUASH,
                e: 0.0,
                f: w * (1.0 + ECHO_SQUASH),
            };
            p.layer(flip, 1.0, |q| {
                for g in &glyphs {
                    let wobble = if still {
                        0.0
                    } else {
                        ECHO_WOBBLE
                            * s
                            * (std::f64::consts::TAU * t / ECHO_WOBBLE_SECS
                                + 0.07 * (g.pos.x / s) as f64)
                                .sin() as f32
                    };
                    q.glyph(GlyphInstance {
                        pos: Point::new(r.x + x + g.pos.x, r.y + top + g.pos.y),
                        size,
                        font: g.font,
                        glyph: g.glyph,
                        word: NO_WORD,
                        color: hex(WHITE).fade(ECHO_ALPHA),
                        xform: GlyphXform {
                            offset: Point::new(wobble, 0.0),
                            ..GlyphXform::IDENTITY
                        },
                    });
                }
            });
            if still {
                return;
            }
            for (age, i) in echo_rings(&borns, t) {
                let (_, cx, level) = words[i];
                let (reach, alpha) = echo_ring(age, level);
                let (rx, ry) = (reach * s, reach * RING_FLAT * s);
                let pts: Vec<Point> = (0..48)
                    .map(|k| {
                        let u = std::f32::consts::TAU * k as f32 / 48.0;
                        Point::new(r.x + cx + rx * u.cos(), r.y + ring_y + ry * u.sin())
                    })
                    .collect();
                p.stroke(
                    &Contour::polyline(&pts, true),
                    &StrokeStyle::solid(1.2 * s, hex(CYAN).fade(alpha)).glow(2.0 * s),
                );
            }
        })
        .size(surface.w, surface.h);
        Some(at(canvas, 0.0, 0.0))
    }

    /// R2-04-c: a soft cone of light from above onto the newest word,
    /// gliding from the word before. Drawn behind the text.
    fn sw_spotlight(&self, text_top: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(TYPEWRITER_SPOTLIGHT) || !self.sw_live_row() {
            return None;
        }
        let row = self.sw_newest_row(text_top)?;
        let s = self.scale();
        let still = self.still();
        let newest = self.shown.last()?;
        let b = row.boxes.iter().find(|b| b.id == newest.id)?;
        let born = self.heard.word(newest.id).map_or(0.0, |h| h.born);
        let to = (row.x + b.rect.x + b.rect.w / 2.0, b.rect.w);
        // From the word before on this row, or from where it is.
        let from = row
            .boxes
            .iter()
            .rev()
            .nth(1)
            .map_or(to, |p| (row.x + p.rect.x + p.rect.w / 2.0, p.rect.w));
        let foot = row.top + b.baseline + row.descent;
        let middle = row.top + b.rect.y + 0.5 * b.rect.h;
        let head = row.top + b.rect.y - CONE_RISE * s;
        let until = if still { 0.0 } else { born + CONE_GLIDE + 0.05 };
        let surface = Self::synthwave_surface(l);
        let canvas = Element::canvas("spotlight", until, move |t, r, p| {
            let k = if still {
                1.0
            } else {
                let u = ((t - born) / CONE_GLIDE).clamp(0.0, 1.0) as f32;
                1.0 - (1.0 - u) * (1.0 - u)
            };
            let cx = r.x + from.0 + (to.0 - from.0) * k;
            let half = 0.5 * (from.1 + (to.1 - from.1) * k) + 14.0 * s;
            let (top, mid, bottom) = (r.y + head, r.y + middle, r.y + foot);
            let light = lerp(hex(WHITE), hex(VIOLET), 0.25);
            // Nested cones soften the sides: the middle gets all four.
            for layer in 1..=4 {
                let w = half * (0.4 + 0.15 * layer as f32);
                let at_mid = 2.0 * s + (w - 2.0 * s) * (mid - top) / (bottom - top);
                let a = CONE_ALPHA / 4.0;
                // Brightest at the word's middle, fading up to the apex
                // and down to under the descenders.
                let piece =
                    |y0: f32, w0: f32, y1: f32, w1: f32, a0: f32, a1: f32, p: &mut Painter| {
                        let quad = Contour::polyline(
                            &[
                                Point::new(cx - w0, y0),
                                Point::new(cx + w0, y0),
                                Point::new(cx + w1, y1),
                                Point::new(cx - w1, y1),
                            ],
                            true,
                        );
                        p.fill(
                            [&quad],
                            &StrokeStyle::solid(1.0, light).linear(
                                Point::new(cx, y0),
                                light.fade(a0),
                                Point::new(cx, y1),
                                light.fade(a1),
                            ),
                        );
                    };
                piece(top, 2.0 * s, mid, at_mid, 0.0, a, p);
                piece(mid, at_mid, bottom, w, a, 0.0, p);
            }
        })
        .size(surface.w, surface.h);
        Some(at(canvas, 0.0, 0.0))
    }

    /// R2-04-b: the small glass prism the waveform runs into.
    fn sw_prism(&self) -> Option<Element<Msg>> {
        if !self.look.on(PRISM_FOCUS) || self.cancelled() || self.extra.final_at.is_some() {
            return None;
        }
        let s = self.scale();
        let glass = Glass::prism(GlassKind::Dark);
        let glass = if self.still() { glass.still() } else { glass };
        let prism = Element::panel("prism")
            .size(PRISM_W * s, PRISM_H * s)
            .glass(glass)
            .presence(Transition::fade(), Transition::fade());
        Some(at(
            prism,
            (PRISM_X - PRISM_W / 2.0) * s,
            (WAVE_Y - PRISM_H / 2.0) * s,
        ))
    }

    /// MO-68: the tube's light runs from the left edge to the newest word,
    /// strokes a line under it, holds and fades. `text_top` is the text
    /// column's top. Under reduced motion the line sits still under the
    /// word once it lands.
    fn sw_light_line(&self, text_top: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(tron::LIGHT_LINE)
            || self.edit_view.is_some()
            || self.ended()
            || self.extra.finish_at.is_some()
        {
            return None;
        }
        let newest = self.shown.last()?;
        let heard = self.heard.word(newest.id)?;
        let row = self.sw_newest_row(text_top)?;
        let b = row.boxes.iter().find(|b| b.id == newest.id)?;
        let (x0, x1) = (row.x + b.rect.x, row.x + b.rect.x + b.rect.w);
        let y = row.top + b.baseline + row.descent + LIGHT_GAP;
        let edge = MARGIN + 2.0 * self.scale();
        let from = heard.onset.unwrap_or(heard.born);
        let landed = heard.born.max(from + LIGHT_LEAVE);
        let reduced = l.on("reduced-motion") || kit::reduced();
        let k = self.scale();
        let surface = Self::synthwave_surface(l);
        let until = if reduced {
            landed
        } else {
            from + LIGHT_LEAVE + LIGHT_DRAW + LIGHT_HOLD + LIGHT_FADE
        };
        let pink = hex(MAGENTA);
        let canvas = Element::canvas(
            format!("light-line-{}", newest.id),
            until,
            move |t, r, p| {
                let line = |p: &mut Painter, a: f32, b: f32, alpha: f32| {
                    if b <= a || alpha <= 0.0 {
                        return;
                    }
                    let ends = [Point::new(r.x + a, r.y + y), Point::new(r.x + b, r.y + y)];
                    p.stroke(
                        &Contour::polyline(&ends, false),
                        &StrokeStyle::solid(1.6 * k, pink.fade(alpha)).glow(5.0 * k),
                    );
                };
                if reduced {
                    if t >= landed {
                        line(p, x0, x1, 0.7);
                    }
                    return;
                }
                let u = t - from;
                if u < 0.0 {
                    return;
                }
                let ease = |f: f64| (1.0 - (1.0 - f.clamp(0.0, 1.0)).powi(2)) as f32;
                let (head, fade) = if u < LIGHT_LEAVE {
                    (edge + (x0 - edge) * ease(u / LIGHT_LEAVE), 1.0)
                } else if u < LIGHT_LEAVE + LIGHT_DRAW {
                    (x0 + (x1 - x0) * ease((u - LIGHT_LEAVE) / LIGHT_DRAW), 1.0)
                } else {
                    let f = (u - LIGHT_LEAVE - LIGHT_DRAW - LIGHT_HOLD) / LIGHT_FADE;
                    (x1, 1.0 - f.clamp(0.0, 1.0) as f32)
                };
                // The lead from the tube, fainter, and the stroke under the
                // word. No bright head: its glow would reach the letters.
                line(p, edge, head.min(x0), 0.35 * fade);
                line(p, x0, head, 0.95 * fade);
            },
        )
        .size(surface.w, surface.h);
        Some(at(canvas, 0.0, 0.0))
    }

    /// Icon, state and detail, centred near the bottom edge.
    fn sw_status(&self, card_h: f32) -> Element<Msg> {
        let l = &self.look;
        let s = self.scale();
        let f = fonts(&l.fonts);
        let status = self.status();
        let px = STATUS_PX * s;
        let line = l.fonts.line_height(f.status, px).unwrap_or(px * 1.3);
        let style = TextStyle::new(f.status, px, line);
        let blue = hex(BLUE);
        let state = if self.extra.final_at.is_some() && !self.cancelled() {
            self.delivery_word().to_string()
        } else {
            status.state.clone()
        };
        // Keyed once, not by state: a new key per state faded the old word
        // out under the new one ("Loading" over "Listening").
        let label = Element::label("state", state.as_str())
            .text_style(style)
            .color(blue)
            .text_halo(Color::hex(BLUE, 0.35), 4.0 * s)
            .presence(
                Transition::fade().offset(0.0, 7.0 * s),
                Transition::fade().offset(0.0, -7.0 * s),
            );
        let detail = status
            .detail
            .trim_start_matches(|c: char| c.is_whitespace() || c == '·')
            .to_string();
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .gap(10.0 * s)
            .child(self.sw_icon(&status))
            .child(label);
        // While a model loads, three dots light one by one after the label.
        if status.loading {
            let dots = (0..3).map(|i| {
                Element::label(format!("dot-{i}"), ".")
                    .text_style(style)
                    .color(blue)
                    .opacity(0.0)
                    .ambient(LoopProp::Opacity, Loop::step_on(i, 3, DOTS_SECS))
            });
            row = row.child(
                Element::row("loading-dots")
                    .margin_edges(0.0, 0.0, 0.0, -8.0 * s)
                    .children(dots),
            );
        }
        // Once pasted, the finishing detail ("0 s of audio left") is stale.
        let pasted = self.extra.final_at.is_some();
        if !detail.is_empty() && !self.cancelled() && !pasted {
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
            row = row
                .child(
                    Element::shape("sep", ShapeKind::Capsule)
                        .size(4.0 * s, 4.0 * s)
                        .fill(blue)
                        .margin_edges(0.0, 4.0 * s, 0.0, 4.0 * s)
                        // Its x follows the state word's width.
                        .motion(Prop::Position, Motion::Snap),
                )
                .child(
                    Element::live_text(format!("detail-{state}"), &words)
                        .text_style(style)
                        .color(blue)
                        .text_halo(Color::hex(BLUE, 0.35), 4.0 * s)
                        .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                        .motion(Prop::Offset, Spring::POP)
                        .word_enter(Transition::fade().offset(0.0, 7.0 * s))
                        // No presence, so a dropped line goes at once: an old line fading out
                        // sat under the new state's words, which still fade in one by one.
                        .word_change(Transition::fade().opacity(0.2).offset(0.0, -7.0 * s)),
                );
        }
        let row_h = 30.0 * s;
        at(
            Element::row("status-row")
                .size(l.width, row_h)
                .justify(JustifyContent::CENTER)
                .align_items(AlignItems::CENTER)
                .child(row),
            0.0,
            card_h - STATUS_UP * s - row_h / 2.0,
        )
        .motion(Prop::Position, Spring::critical_to_98(GROW_SECS))
    }

    /// Microphone while recording; a stop square in a spinning ring while
    /// the final text is made; a check in a ring when done; an X when
    /// cancelled.
    fn sw_icon(&self, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static CROSS: OnceLock<Arc<Outline>> = OnceLock::new();
        let s = self.scale();
        let side = 30.0 * s;
        let icon_view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let ring_view = Rect::new(0.0, 0.0, 26.0, 26.0);
        let ring = RING.get_or_init(|| {
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(13.0, 13.0), 11.5)],
            })
        });
        let style = |w: f32| {
            StrokeStyle::solid(w, hex(MAGENTA))
                .linear(
                    Point::new(0.0, 0.0),
                    hex(MAGENTA),
                    Point::new(0.0, side),
                    hex(VIOLET),
                )
                .glow(1.0)
        };
        let slot = Element::layer("icon")
            .size(side, side)
            .blend(BlendMode::Add)
            .bloom(0.3, 3.0);
        let done = self.extra.final_at.is_some();
        let stopped = !done && (status.finishing || self.extra.finish_at.is_some());
        if self.cancelled() {
            let cross = CROSS.get_or_init(|| Arc::new(Outline::new(&icons::x(), 0.05)));
            return slot.child(at(
                Element::stroke_path("x", cross.clone(), icon_view, style(2.0))
                    .size(side * 0.7, side * 0.7)
                    .motion(Prop::Trim, Spring::critical_to_98(0.3))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                side * 0.15,
                side * 0.15,
            ));
        }
        if done {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            return slot
                .child(at(
                    Element::stroke_path("ring-closed", ring.clone(), ring_view, style(1.6))
                        .size(side, side)
                        .motion(Prop::Trim, Spring::critical_to_98(0.25))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    0.0,
                ))
                .child(at(
                    Element::stroke_path("check", check.clone(), icon_view, style(2.4))
                        .size(side * 0.55, side * 0.55)
                        .motion(Prop::Trim, Spring::critical_to_98(0.5))
                        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    side * 0.225,
                    side * 0.225,
                ));
        }
        let mut slot = slot;
        if stopped && !self.ended() {
            slot = slot.child(at(
                Element::stroke_path(
                    "ring-spin",
                    ring.clone(),
                    ring_view,
                    StrokeStyle::solid(1.2, hex(VIOLET).fade(0.4)),
                )
                .size(side, side)
                .light_head(0.3, 0.8, hex(CYAN), 3.0)
                .presence(Transition::fade().scale(0.6), Transition::fade()),
                0.0,
                0.0,
            ));
        }
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 64))
        });
        let icon = if stopped { side * 0.4 } else { side };
        slot.child(at(
            Element::morph("mic", morph.clone(), icon_view, style(1.8))
                .size(icon, icon)
                .morph_to(if stopped { 1.0 } else { 0.0 })
                .motion(Prop::Morph, Spring::critical_to_98(0.3))
                .presence(Transition::fade(), Transition::fade().scale(0.6)),
            (side - icon) / 2.0,
            (side - icon) / 2.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_onset_sends_one_row_from_the_horizon() {
        let run = |t: f64| t;
        let onsets = [1.0, 1.4, 5.0];
        // At 2 s: two rows, the newer one further away; the third onset has
        // not come yet.
        let d = onset_depths(&onsets, 2.0, 2.0, run);
        assert_eq!(d.len(), 2);
        assert!(d[0] < d[1] && d[1] < ROW_FAR);
        // A row starts at the horizon and crosses the road in the travel time.
        let t = 3.0 + ONSET_TRAVEL_SECS;
        let d = onset_depths(&[3.0], t, t, run);
        assert!((d[0] - ROW_NEAR).abs() < 1e-5);
        // Silence: no onsets, an empty road; old rows have left it.
        assert!(onset_depths(&[], 9.0, 9.0, run).is_empty());
        assert!(onset_depths(&[1.0], 9.0, 9.0, run)[0] < ROW_NEAR);
    }

    #[test]
    fn idle_rows_wait_for_an_empty_road() {
        // Open with no speech: in after a moment.
        assert_eq!(idle_share(&[], 0.2, 0.0), 0.0);
        assert_eq!(idle_share(&[], 3.0, 0.0), 1.0);
        // Speech: out within the fade, back once the last row has crossed.
        let o = [1.0, 1.5];
        assert!(idle_share(&o, 1.6, 0.0) < 0.05);
        assert_eq!(idle_share(&o, 1.5 + ONSET_TRAVEL_SECS, 0.0), 0.0);
        assert_eq!(idle_share(&o, 1.5 + ONSET_TRAVEL_SECS + 1.0, 0.0), 1.0);
        // An onset after a long silence fades the rows out, not at once.
        let late = [10.0];
        assert!(idle_share(&late, 10.1, 0.0) > 0.3);
        assert_eq!(idle_share(&late, 10.4, 0.0), 0.0);
    }

    #[test]
    fn reduced_motion_rows_stand_still_between_onsets() {
        let run = |t: f64| t;
        // Seen at 3 s and 4 s with the rows as they stood at the last onset.
        let a = onset_depths(&[1.0, 2.0], 3.0, 2.0, run);
        let b = onset_depths(&[1.0, 2.0], 4.0, 2.0, run);
        assert_eq!(a, b);
    }
}

/// Synthwave's edit adapter (plan section 4.5): one row per window line,
/// bottom-anchored, and the onset grid still.
pub(crate) struct SynthwaveSite;

impl edit_site::EditSite for SynthwaveSite {
    fn quiet(&self) -> &'static [&'static str] {
        &[ONSET_GRID]
    }

    fn place(&self, look: &Look, el: Element<Msg>, view: &edit::EditView) -> Element<Msg> {
        edit_site::place_lines(self, look, el, view)
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::scope::edit_tests::check_edit_envelope;
    use super::super::swiss_look::edit_check::Rig;
    use super::*;

    /// M7.L12: Synthwave edits in its own card. With the caret on the
    /// newest, a middle and the top line, at both widths: the caret on its
    /// glyph within 1 px in every frame, the popup's size unchanged, and
    /// the golden `tests/fixtures/osd-vo/edit-synthwave.txt`.
    #[test]
    fn synthwave_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Synthwave, "synthwave");
    }

    /// While editing the rows are the window's lines, the newest at the
    /// bottom, and the onset grid is still.
    #[test]
    fn synthwave_rows_are_the_window() {
        let mut r = Rig::new(OsdTheme::Synthwave, false);
        r.dictate(160);
        r.enter(1.0);
        assert!(!r.p.look.on(ONSET_GRID));
        let v = r.p.edit_view.clone().unwrap();
        let mut rows = Vec::new();
        let _ = r.p.view(&vo::Tokens::default()).map_live_texts(&mut |e| {
            if e.key().to_string().starts_with("line-") {
                rows.push(e.live_words().map(|w| w.len()).unwrap_or(0));
            }
            e
        });
        assert_eq!(rows.len(), v.lines as usize);
    }

    /// R35: the edit moment with the caret on each line stays inside the
    /// reading-line envelope, at both widths.
    #[test]
    fn synthwave_edit_moment_keeps_the_envelope() {
        check_edit_envelope(OsdTheme::Synthwave, &[0, 1, 2]);
    }
}

#[cfg(test)]
mod round2_tests {
    use super::*;

    #[test]
    fn the_dip_stays_within_2_px_and_settles_in_120_ms() {
        let enter = dip_enter(false);
        let (_, dy) = enter.offset.expect("a dip");
        assert!(dy > 0.0 && dy <= 2.0);
        assert_eq!(enter.opacity, Some(DIP_INK));
        let spring = Spring::critical_to_98(DIP_SECS);
        // No overshoot on the way back, and back by 120 ms.
        for i in 0..=60 {
            let y = spring.eval(dy as f64, 0.0, i as f64 * 0.005).0;
            assert!((-1e-3..=dy as f64 + 1e-6).contains(&y), "{y} at {i}");
        }
        assert!(spring.eval(dy as f64, 0.0, DIP_SECS).0.abs() <= 0.02 * dy as f64 + 1e-6);
        // Reduced motion: no dip.
        assert_eq!(dip_enter(true).offset, None);
    }

    #[test]
    fn one_ring_per_word() {
        let borns = [1.0, 1.3, 2.5];
        assert_eq!(echo_rings(&borns, 0.9).len(), 0);
        assert_eq!(echo_rings(&borns, 1.06), vec![(1.06 - 1.0, 0)]);
        assert_eq!(echo_rings(&borns, 1.5).len(), 2);
        // Each ring is gone a second after its word.
        assert_eq!(echo_rings(&borns, 2.31).len(), 0);
        assert_eq!(echo_rings(&borns, 3.6).len(), 0);
        for age in [0.0, 0.3, 0.7, 0.999] {
            let (reach, alpha) = echo_ring(age, 1.0);
            assert!(reach <= RING_REACH.1 && alpha <= RING_ALPHA);
        }
    }

    #[test]
    fn the_fan_follows_the_voice_alone() {
        assert_eq!(fan_len(0.0), FAN_LEN.0);
        assert_eq!(fan_len(1.0), FAN_LEN.1);
        let mut last = 0.0;
        for i in 0..=20 {
            let rms = i as f32 / 20.0;
            assert_eq!(fan_len(rms), fan_len(rms));
            assert!(fan_len(rms) >= last);
            last = fan_len(rms);
            assert!(FAN_ALPHA * fan_power(rms) <= 0.3 + 1e-6);
        }
        // Room tone is no voice.
        let levels = kit::Levels {
            levels: vec![0.02; 30].into(),
            newest_at: 1.0,
        };
        assert_eq!(fan_level(&levels, 1.0), 0.0);
    }
}
