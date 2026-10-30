//! The `cyberpunk` look of the vo popup, after mock-up 11: a near-black
//! chamfered panel in a yellow and red frame, a yellow tag that types
//! "VOICE LINK ACTIVE", hazard slashes, bars that mirror out from the
//! centre between brackets, bold white words and a chamfered status bar.
//!
//! The frame, the bars and the decoration are drawn on canvases from the
//! frame time; the text is labels and live text. Glitch slices fire only on
//! events (the open, a revised word, a change of state), so an idle popup
//! draws nothing new. Short ticks drive the typing and the panel's growth.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::*;

use std::sync::OnceLock;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::flow::{Revise, ROLL_WORD_EMS};
use vo::scene::taffy;
use vo::scene::Insets;
use vo::scene::taffy::prelude::{auto, length};
use vo_core::words::WordId;

/// Colours sampled from the mock-up.
const YELLOW: u32 = 0xFCEA2E;
const RED: u32 = 0xFA232B;
const MAGENTA: u32 = 0xF9194C;
const PINK: u32 = 0xFF2E88;
const ORANGE: u32 = 0xFF8A1F;
const CYAN: u32 = 0x19E6F0;
const BODY: u32 = 0x0B0D11;
const INK: u32 = 0x15130C;
const PAPER: u32 = 0xF4F6FA;
const GREY: u32 = 0x5E636B;

fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

/// Linear mix of two colours.
fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

/// The panel's design width: every x below is in these units.
const DESIGN_W: f32 = 600.0;
/// The narrowest panel the design holds together at.
const MIN_WIDTH: f32 = 560.0;
/// Room around the panel for the glitch slices that stick out of it.
const MARGIN_X: f32 = 20.0;
const MARGIN_Y: f32 = 6.0;
/// The transcript: Inter SemiBold, as large as in the mock-up.
const TEXT_PX: f32 = 30.5;
const TEXT_LINE: f32 = 37.5;
const TEXT_X: f32 = 66.0;
const TEXT_TOP: f32 = 146.0;
/// Width the transcript wraps at, in design units.
const WRAP: f32 = 482.0;
/// Below the transcript: gap, status bar, and the frame under it.
const STATUS_GAP: f32 = 13.0;
const STATUS_H: f32 = 45.0;
const STATUS_BOTTOM: f32 = 19.0;
/// The waveform: centre line, half height and bar count.
const WAVE_Y: f32 = 110.0;
const WAVE_HALF: f32 = 31.0;
const WAVE_X0: f32 = 65.0;
const WAVE_X1: f32 = 528.0;
const BARS: usize = 44;
/// Seconds of audio between neighbouring bars.
const BAR_SECS: f64 = 0.045;
/// The tag's text starts here; the tag ends this far past it.
const TAG_TEXT_X: f32 = 65.0;
const TAG_PAD: f32 = 32.0;
const TAG_PX: f32 = 17.5;

/// Seconds for the frame to draw on, and between its corners.
const DRAW_SECS: f64 = 0.30;
const DRAW_STAGGER: f64 = 0.04;
/// Seconds per character typed, and per character deleted.
const TYPE_SECS: f64 = 0.012;
const DELETE_SECS: f64 = 0.004;
/// The first typing waits for the tag to wipe in.
const TYPE_DELAY: f64 = 0.12;
/// Seconds the cursor blinks after the typing: two blinks.
const BLINK_SECS: f64 = 0.48;
/// Seconds for the panel to grow by a line, and for the tag to resize.
const GROW_SECS: f64 = 0.15;
/// A glitch burst, and the least time between two from revisions.
const BURST_SECS: f64 = 0.14;
const BURST_GAP: f64 = 0.4;
/// Seconds a revised word's old text takes to roll up and out.
const ROLL_SECS: f64 = 0.22;
/// The open's glitch.
const OPEN_GLITCH_SECS: f64 = 0.24;
/// Ticks while something is being typed or grown.
const TICK_HZ: f64 = 60.0;

/// The fonts the look adds to the registry.
#[derive(Clone, Copy)]
struct Fonts {
    text: u32,
    body: u32,
    tag: u32,
    mono: u32,
}

/// The look's fonts, added to the shared registry once. A family that is
/// missing falls back to the popup's own fonts.
fn fonts(reg: &FontRegistry) -> Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    *FONTS.get_or_init(|| {
        let add = |pattern: &str, or: u32| reg.add_system(pattern).unwrap_or(or);
        Fonts {
            text: add("Inter:semibold", SANS),
            body: add("Inter", SANS),
            tag: add("Barlow:bold", SANS),
            mono: add("JetBrains Mono:bold", MONO),
        }
    })
}

/// Reduced motion, as the framework reads it: no slice jitter, no typing,
/// no drift.
fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        std::env::var("VO_REDUCED_MOTION").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
    })
}

/// The transcript: Inter SemiBold, as large as in the mock-up.
pub(crate) fn text_style(look: &Look) -> TextStyle {
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX, TEXT_LINE).tracking(-0.3)
}

/// The panel's width: the configured one, but never so narrow that the tag,
/// the status bar and the count box collide.
fn width(look: &Look) -> f32 {
    look.width.max(MIN_WIDTH)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    WRAP * width(look) / DESIGN_W
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

fn ease_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (1.0 - (1.0 - k).powi(3)) as f32
}

fn ease_in_out(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    (if k < 0.5 {
        4.0 * k * k * k
    } else {
        1.0 - (-2.0 * k + 2.0).powi(3) / 2.0
    }) as f32
}

/// An ease out that overshoots a little, for things that snap into place.
fn ease_back(k: f64) -> f32 {
    let k = k.clamp(0.0, 1.0);
    let c = 1.70158;
    (1.0 + (c + 1.0) * (k - 1.0).powi(3) + c * (k - 1.0).powi(2)) as f32
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFFFF) as f32 / 65535.0
}

/// What the popup shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Loading,
    Listening,
    Finishing,
    Done,
    Cancelled,
}

impl Mode {
    fn tag(self) -> &'static str {
        match self {
            Mode::Loading => "VOICE LINK BOOTING",
            Mode::Listening => "VOICE LINK ACTIVE",
            Mode::Finishing => "VOICE LINK PROCESSING",
            Mode::Done => "TRANSMISSION SENT",
            Mode::Cancelled => "VOICE LINK SEVERED",
        }
    }
}

/// State only the cyberpunk look keeps.
pub struct State {
    mode: Option<Mode>,
    /// When the mode last changed.
    mode_at: f64,
    /// The tag's text before the last change, and the change's time.
    old_tag: &'static str,
    tag_at: f64,
    /// Transcript lines before the last growth, and its time.
    old_lines: u32,
    lines: u32,
    lines_at: f64,
    /// The last glitch burst from a revised word.
    revised_at: f64,
    /// The word that was revised.
    revised: Option<WordId>,
    /// The words on screen at the last sync.
    words: Vec<Word>,
    /// Revised words' old text rolling up and out: the word, its old
    /// text and when it was revised.
    rolls: Vec<(WordId, String, f64)>,
}

impl Default for State {
    fn default() -> State {
        State {
            mode: None,
            mode_at: f64::NEG_INFINITY,
            old_tag: "",
            tag_at: 0.0,
            old_lines: 1,
            lines: 1,
            lines_at: f64::NEG_INFINITY,
            revised_at: f64::NEG_INFINITY,
            revised: None,
            words: Vec::new(),
            rolls: Vec::new(),
        }
    }
}

/// The panel's height with `lines` transcript lines.
fn panel_h(lines: f32) -> f32 {
    TEXT_TOP + lines * TEXT_LINE + STATUS_GAP + STATUS_H + STATUS_BOTTOM
}

/// The panel at its tallest. Its bottom stays here and it grows upward.
fn tall_h() -> f32 {
    panel_h(swiss::MAX_LINES as f32)
}

/// The centre of the newest transcript line, from the surface top. The panel
/// grows upward as lines are added, so this does not depend on the count.
pub(crate) fn newest_line_centre() -> f32 {
    MARGIN_Y + tall_h() - (STATUS_GAP + STATUS_H + STATUS_BOTTOM) - TEXT_LINE / 2.0
}

/// The text a typewriter shows `since` seconds after it began to replace
/// `old` with `new`, and whether its cursor is lit.
fn typed(old: &str, new: &str, since: f64) -> (String, bool) {
    if reduced() {
        return (new.to_string(), false);
    }
    if since < 0.0 {
        return (old.to_string(), false);
    }
    let delete = old.len() as f64 * DELETE_SECS;
    if since < delete {
        let gone = (since / DELETE_SECS) as usize;
        return (old[..old.len() - gone.min(old.len())].to_string(), true);
    }
    let k = (((since - delete) / TYPE_SECS) as usize + 1).min(new.len());
    let typed_end = delete + new.len() as f64 * TYPE_SECS;
    let lit = if since < typed_end {
        true
    } else {
        let blink = since - typed_end;
        blink < BLINK_SECS && (blink / (BLINK_SECS / 4.0)) as u32 % 2 == 1
    };
    (new[..k].to_string(), lit)
}

/// Seconds a typewriter takes to replace `old` with `new` and blink.
fn typing_secs(old: &str, new: &str) -> f64 {
    old.len() as f64 * DELETE_SECS + new.len() as f64 * TYPE_SECS + BLINK_SECS
}

/// The panel's outline pieces at height `h` and width `w`, in panel space.
struct Geo {
    w: f32,
    h: f32,
}

impl Geo {
    /// An x measured from the panel's right edge in design units.
    fn r(&self, x: f32) -> f32 {
        self.w - (DESIGN_W - x)
    }

    fn status_y(&self) -> f32 {
        self.h - STATUS_BOTTOM - STATUS_H
    }

    /// The near-black body.
    fn body(&self) -> Vec<(f32, f32)> {
        let (h, r) = (self.h, |x| self.r(x));
        vec![
            (12.0, 58.0),
            (38.0, 32.0),
            (384.0, 32.0),
            (412.0, 3.0),
            (r(578.0), 3.0),
            (r(596.0), 21.0),
            (r(596.0), h - 61.0),
            (r(556.0), h - 1.0),
            (46.0, h - 1.0),
            (12.0, h - 35.0),
        ]
    }

    /// Stroked pieces, each drawn on from its first point: points, width,
    /// colour key and draw-on order.
    fn pieces(&self, tag_r: f32) -> Vec<Piece> {
        let (h, r) = (self.h, |x| self.r(x));
        let sy = self.status_y();
        vec![
            Piece::new(
                &[(38.0, 32.0), (384.0, 32.0), (412.0, 3.0), (r(560.0), 3.0)],
                3.0,
                Hue::Yellow,
                0,
            ),
            Piece::new(
                &[
                    (38.0, 32.0),
                    (12.0, 58.0),
                    (12.0, h - 95.0),
                    (24.0, h - 71.0),
                    (24.0, h - 23.0),
                    (46.0, h - 1.0),
                    (300.0, h - 1.0),
                ],
                3.0,
                Hue::Fade,
                0,
            ),
            Piece::new(
                &[
                    (r(560.0), 3.0),
                    (r(578.0), 3.0),
                    (r(596.0), 21.0),
                    (r(596.0), 58.0),
                    (r(588.0), 66.0),
                ],
                4.5,
                Hue::Magenta,
                1,
            ),
            Piece::new(&[(r(588.0), 66.0), (r(588.0), h - 128.0)], 2.0, Hue::Red, 1),
            Piece::new(
                &[
                    (r(593.0), h - 128.0),
                    (r(593.0), h - 62.0),
                    (r(553.0), h - 20.0),
                    (r(478.0), h - 20.0),
                ],
                4.0,
                Hue::Yellow,
                2,
            ),
            Piece::new(
                &[(r(584.0), h - 120.0), (r(584.0), h - 68.0), (r(548.0), h - 30.0)],
                2.0,
                Hue::Red,
                2,
            ),
            // The inner echo above the waveform, from the tag's end.
            Piece::new(
                &[
                    (tag_r - 16.0, 63.0),
                    (tag_r + 7.0, 40.0),
                    (437.0f32.max(tag_r + 30.0), 40.0),
                    (463.0f32.max(tag_r + 56.0), 13.0),
                    (r(572.0), 13.0),
                ],
                1.2,
                Hue::Yellow,
                1,
            ),
            Piece::new(&[(24.0, 66.0), (24.0, 106.0)], 1.2, Hue::Yellow, 0),
            // The status bar: a pointed left end and a stepped right one.
            Piece::new(
                &[
                    (40.0, sy + STATUS_H / 2.0),
                    (52.0, sy),
                    (r(490.0), sy),
                    (r(500.0), sy + 10.0),
                    (r(458.0), sy + 36.0),
                    (r(402.0), sy + 36.0),
                    (r(394.0), sy + STATUS_H),
                    (52.0, sy + STATUS_H),
                    (40.0, sy + STATUS_H / 2.0),
                ],
                1.4,
                Hue::Yellow,
                3,
            ),
            // The count box.
            Piece::new(
                &[
                    (r(456.0), h - 6.0),
                    (r(499.0), sy + 5.0),
                    (r(578.0), sy + 5.0),
                    (r(533.0), h - 6.0),
                    (r(456.0), h - 6.0),
                ],
                1.5,
                Hue::Red,
                3,
            ),
        ]
    }
}

/// A frame colour, which turns red when the dictation is cancelled.
#[derive(Clone, Copy)]
enum Hue {
    Yellow,
    Red,
    Magenta,
    /// Yellow at the top fading to red at the bottom.
    Fade,
}

struct Piece {
    pts: Vec<Point>,
    width: f32,
    hue: Hue,
    order: u32,
}

impl Piece {
    fn new(pts: &[(f32, f32)], width: f32, hue: Hue, order: u32) -> Piece {
        Piece {
            pts: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
            width,
            hue,
            order,
        }
    }
}

fn poly(pts: &[(f32, f32)], o: Point) -> Contour {
    let pts: Vec<Point> = pts
        .iter()
        .map(|&(x, y)| Point::new(o.x + x, o.y + y))
        .collect();
    Contour::polyline(&pts, false)
}

/// A glitch slice that rests outside the frame: side (0 left, 1 right),
/// height as a fraction of the panel, start, length, thickness, colour.
const SLICES: [(u8, f32, f32, f32, f32, u32); 16] = [
    (0, 0.196, -12.0, 38.0, 2.2, 0xFFFFFF),
    (0, 0.244, -6.0, 16.0, 1.5, PINK),
    (0, 0.349, 0.0, 26.0, 2.6, 0xE8FFE0),
    (0, 0.360, 10.0, 12.0, 1.6, CYAN),
    (0, 0.400, -8.0, 22.0, 1.4, RED),
    (0, 0.477, -4.0, 30.0, 2.4, 0xFFFFFF),
    (0, 0.515, 2.0, 14.0, 2.0, YELLOW),
    (0, 0.580, -10.0, 18.0, 1.4, PINK),
    (0, 0.640, 4.0, 20.0, 2.6, 0x9CFF5A),
    (0, 0.910, -6.0, 26.0, 1.8, RED),
    (1, 0.300, -14.0, 22.0, 1.4, RED),
    (1, 0.625, -18.0, 30.0, 2.4, YELLOW),
    (1, 0.670, -6.0, 22.0, 1.6, RED),
    (1, 0.718, -10.0, 34.0, 2.2, 0xFFFFFF),
    (1, 0.765, -2.0, 18.0, 1.4, CYAN),
    (1, 0.810, -12.0, 26.0, 2.0, RED),
];

/// How far the decoration canvas draws past the surface: the right slices
/// reach 24 px past the frame and a burst shakes them 15 px more, frame
/// pieces jitter 14 px, and the frame's 2 px glow fades out over about 12 px.
const DECO_REACH: Insets = Insets {
    left: 8.0,
    top: 8.0,
    right: 20.0,
    bottom: 8.0,
};

/// Everything the decoration canvas needs, copied out of the popup.
#[derive(Clone)]
struct Deco {
    w: f32,
    shown_at: f64,
    /// Panel height now and the growth in progress.
    h0: f32,
    h1: f32,
    grow_at: f64,
    /// Tag end now and the resize in progress.
    tag0: f32,
    tag1: f32,
    tag_at: f64,
    mode: Mode,
    mode_at: f64,
    /// Glitch bursts: (start, seconds, strength).
    bursts: Vec<(f64, f64, f32)>,
    /// The final-pass progress, 0 to 1, when there is one.
    progress: Option<f32>,
    /// The revised word's box in panel space and when it was revised.
    mark: Option<(f64, Rect)>,
}

impl Deco {
    fn h(&self, t: f64) -> f32 {
        let k = ease_in_out((t - self.grow_at) / GROW_SECS);
        self.h0 + (self.h1 - self.h0) * k
    }

    fn tag_r(&self, t: f64) -> f32 {
        let k = ease_out((t - self.tag_at) / GROW_SECS);
        self.tag0 + (self.tag1 - self.tag0) * k
    }

    /// How red the frame is: 1 once a cancel has turned it.
    fn red(&self, t: f64) -> f32 {
        if self.mode == Mode::Cancelled {
            ease_out((t - self.mode_at) / 0.2)
        } else {
            0.0
        }
    }

    /// The strongest burst running at `t`, 0 when none.
    fn burst(&self, t: f64) -> f32 {
        if reduced() {
            return 0.0;
        }
        self.bursts
            .iter()
            .filter(|(at, secs, _)| t >= *at && t < at + secs)
            .map(|(at, secs, s)| s * (1.0 - ((t - at) / secs) as f32 * 0.5))
            .fold(0.0, f32::max)
    }

    /// The time after which nothing it draws changes.
    fn until(&self) -> f64 {
        let open = self.shown_at + DRAW_SECS + 3.0 * DRAW_STAGGER + 0.2;
        let bursts = self
            .bursts
            .iter()
            .map(|(at, secs, _)| at + secs)
            .fold(f64::NEG_INFINITY, f64::max);
        open.max(self.grow_at + GROW_SECS)
            .max(self.tag_at + GROW_SECS)
            .max(self.mode_at + 0.25)
            .max(bursts)
    }

    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let h = self.h(t);
        let o = Point::new(rect.x + MARGIN_X, rect.y + MARGIN_Y + tall_h() - h);
        let g = Geo { w: self.w, h };
        let red = self.red(t);
        let since = t - self.shown_at;
        let burst = self.burst(t);
        // Bursts shake whole slices of the panel sideways.
        let frame = (t * 30.0) as u32;
        let jitter = |y: f32, salt: u32| -> f32 {
            if burst <= 0.0 {
                return 0.0;
            }
            let band = (y / 9.0) as u32;
            let n = noise(band.wrapping_mul(131) ^ frame.wrapping_mul(7919) ^ salt);
            if n > 0.72 {
                (noise(band ^ frame ^ 0x55) - 0.5) * 36.0 * burst
            } else {
                0.0
            }
        };

        // The body.
        let body = Color::hex(BODY, 0.94);
        p.fill(
            [&poly(&g.body(), o)],
            &StrokeStyle::solid(0.0, body),
        );

        let yellow = mix(hex(YELLOW), hex(RED), red);
        let magenta = hex(MAGENTA);
        let colour = |hue: Hue| match hue {
            Hue::Yellow => yellow,
            Hue::Red => hex(RED),
            Hue::Magenta => magenta,
            Hue::Fade => yellow,
        };
        // The frame draws on from its corners.
        let tag_r = self.tag_r(t);
        for piece in g.pieces(tag_r) {
            let k = if reduced() {
                1.0
            } else {
                ease_out((since - piece.order as f64 * DRAW_STAGGER) / DRAW_SECS)
            };
            if k <= 0.0 {
                continue;
            }
            let y = piece.pts[0].y;
            let dx = jitter(y, piece.order);
            let pts: Vec<Point> = piece
                .pts
                .iter()
                .map(|q| Point::new(o.x + q.x + dx, o.y + q.y))
                .collect();
            let c = Contour::polyline(&pts, false);
            let c = if k < 1.0 { c.trim(0.0, c.length() * k) } else { c };
            let style = match piece.hue {
                Hue::Fade => StrokeStyle::solid(piece.width, yellow).linear(
                    Point::new(0.0, o.y + g.h * 0.62),
                    yellow,
                    Point::new(0.0, o.y + g.h - 80.0),
                    hex(RED),
                ),
                hue => StrokeStyle::solid(piece.width, colour(hue)),
            };
            p.stroke(&c, &style.glow(2.0));
        }

        // A red wedge in the bottom right corner, and the broken bottom edge.
        let k3 = ease_out((since - 3.0 * DRAW_STAGGER) / DRAW_SECS);
        if k3 > 0.0 {
            let (h, r) = (g.h, |x| g.r(x));
            p.fill(
                [&poly(
                    &[
                        (r(584.0), h - 104.0),
                        (r(584.0), h - 70.0),
                        (r(566.0), h - 52.0),
                        (r(566.0), h - 86.0),
                    ],
                    o,
                )],
                &StrokeStyle::solid(0.0, hex(RED).fade(k3)),
            );
            let edge = [
                (300.0, 34.0, RED),
                (338.0, 64.0, YELLOW),
                (406.0, 30.0, PINK),
                (440.0, 18.0, YELLOW),
                (r(470.0), 12.0, 0x8A3CFF),
            ];
            for (x, len, c) in edge {
                let c = mix(hex(c), hex(RED), red).fade(k3);
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(o.x + x, o.y + h - 2.5, len, 3.0), 0.0),
                    Paint::solid(c),
                ));
            }
        }

        // The tag: a yellow tab that wipes in from the left.
        let wipe = if reduced() {
            1.0
        } else {
            ease_out((since - 0.06) / 0.18)
        };
        if wipe > 0.0 {
            let right = 41.0 + (tag_r - 41.0) * wipe;
            let tag = [
                (35.0, 41.0),
                (41.0, 35.0),
                (right, 35.0),
                ((right - 24.0).max(41.0), 62.0),
                (39.0, 62.0),
                (35.0, 58.0),
            ];
            p.fill([&poly(&tag, o)], &StrokeStyle::solid(0.0, yellow).glow(3.0));
        }

        // Brackets round the waveform snap out from the centre.
        let snap = if reduced() {
            1.0
        } else {
            ease_back((since - 0.15) / 0.2)
        };
        if snap > 0.0 {
            let mid = (WAVE_X0 + g.r(WAVE_X1)) / 2.0;
            let (l, r) = (48.0, g.r(551.0));
            let lx = mid + (l - mid) * snap;
            let rx = mid + (r - mid) * snap;
            let (y0, y1) = (WAVE_Y - 16.0, WAVE_Y + 17.0);
            let style = StrokeStyle::solid(1.4, yellow).glow(2.0);
            p.stroke(
                &poly(&[(lx + 3.5, y0), (lx, y0 + 3.0), (lx, y1 - 3.0), (lx + 3.5, y1)], o),
                &style,
            );
            p.stroke(
                &poly(&[(rx - 3.5, y0), (rx, y0 + 3.0), (rx, y1 - 3.0), (rx - 3.5, y1)], o),
                &style,
            );
        }

        // Two slashes at the status bar's right end.
        let sy = g.status_y();
        let k = if reduced() {
            1.0
        } else {
            ease_out((since - 0.25) / 0.2)
        };
        if k > 0.0 {
            for i in 0..2 {
                let x = g.r(446.0) + i as f32 * 16.0 + (1.0 - k) * 30.0;
                p.fill(
                    [&poly(
                        &[
                            (x + 13.0, sy + 14.0),
                            (x + 20.0, sy + 14.0),
                            (x + 7.0, sy + 34.0),
                            (x, sy + 34.0),
                        ],
                        o,
                    )],
                    &StrokeStyle::solid(0.0, yellow.fade(k)),
                );
            }
        }

        // The final pass's progress along the bottom of the status bar.
        if let Some(f) = self.progress {
            let x0 = 54.0;
            let x1 = g.r(392.0);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(o.x + x0, o.y + sy + STATUS_H - 5.0, x1 - x0, 2.0), 0.0),
                Paint::solid(hex(GREY).fade(0.6)),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(
                    Rect::new(o.x + x0, o.y + sy + STATUS_H - 5.0, (x1 - x0) * f, 2.0),
                    0.0,
                ),
                Paint::solid(yellow),
            ));
        }

        // A revised word: a yellow bar flashes behind it, and two slices
        // cross the frame at its height.
        if let Some((at, b)) = self.mark {
            let k = (t - at) / (2.0 * BURST_SECS);
            if (0.0..1.0).contains(&k) && !reduced() {
                let a = 1.0 - k as f32;
                let step = ((t - at) * 30.0) as u32;
                let dx = (noise(step * 17) - 0.5) * 8.0;
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(o.x + b.x - 4.0 + dx, o.y + b.y + b.h * 0.2, b.w + 8.0, b.h * 0.62), 0.0),
                    Paint::solid(hex(YELLOW).fade(0.45 * a)),
                ));
                for (i, c) in [(0u32, RED), (1, CYAN)] {
                    let y = o.y + b.y + b.h * (0.35 + 0.3 * i as f32) + (noise(step + i) - 0.5) * 4.0;
                    let jx = (noise(step * 3 + i) - 0.5) * 16.0;
                    // From the left edge to the text, and from the word to the right edge.
                    let right = b.x + b.w + 10.0;
                    for (x0, x1) in [(-8.0, TEXT_X - 8.0), (right, g.w + 8.0)] {
                        p.shape(ShapeInstance::rrect(
                            RRect::new(Rect::new(o.x + x0 + jx, y, (x1 - x0).max(0.0), 1.5), 0.0),
                            Paint::solid(hex(c).fade(0.6 * a)),
                        ));
                    }
                }
            }
        }

        // Glitch slices: a resting set outside the frame that jumps on a
        // burst, and on a burst a few more across the panel.
        let open = if reduced() {
            0.0
        } else {
            (1.0 - since / OPEN_GLITCH_SECS).clamp(0.0, 1.0) as f32
        };
        let shake = burst.max(open);
        for (i, &(side, fy, x, len, thick, c)) in SLICES.iter().enumerate() {
            let y = fy * g.h;
            let base = if side == 0 { x } else { g.w + x };
            let dx = if shake > 0.0 {
                (noise(i as u32 * 97 + frame) - 0.5) * 30.0 * shake
            } else {
                0.0
            };
            let c = mix(hex(c), hex(RED), red * 0.6);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(o.x + base + dx, o.y + y, len, thick), 0.0),
                Paint::solid(c.fade(0.9)),
            ));
        }
        if shake > 0.0 {
            for i in 0..6u32 {
                let n = noise(i * 13 + frame * 31);
                if n < 0.35 {
                    continue;
                }
                let y = noise(i * 7 + frame * 3) * g.h;
                let x = noise(i * 5 + frame * 11) * g.w * 0.8 - 10.0;
                let len = 20.0 + noise(i + frame) * 90.0;
                let c = [RED, CYAN, YELLOW, PINK, PAPER][(i % 5) as usize];
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(o.x + x, o.y + y, len, 1.5 + 2.0 * n), 0.0),
                    Paint::solid(hex(c).fade(0.55 * shake)),
                ));
            }
        }
    }
}

/// What the waveform canvas needs.
#[derive(Clone)]
struct Bars {
    levels: Arc<[f32]>,
    newest_at: f64,
    shown_at: f64,
    mode: Mode,
    finish_at: Option<f64>,
    final_at: Option<f64>,
    progress: Option<f32>,
    /// When the dictation was cancelled.
    cancel_at: Option<f64>,
}

impl Bars {
    /// The smoothed level at time `s`, 0 before the first frame.
    fn level_at(&self, s: f64) -> f32 {
        let n = self.levels.len();
        if n == 0 {
            return 0.0;
        }
        let back = ((self.newest_at - s) * FRAME_HZ as f64).max(0.0);
        let i = (n - 1) as f64 - back;
        if i < 0.0 {
            return 0.0;
        }
        let (a, k) = (i.floor() as usize, (i - i.floor()) as f32);
        let b = (a + 1).min(n - 1);
        self.levels[a] * (1.0 - k) + self.levels[b] * k
    }

    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => now + 0.25,
            Mode::Finishing => now + 0.5,
            _ => self.final_at.unwrap_or(now) + 0.5,
        }
    }

    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let x0 = rect.x;
        let span = rect.w;
        let cy = rect.y + rect.h / 2.0;
        let pitch = span / (BARS - 1) as f32;
        let since = t - self.shown_at;
        if self.mode == Mode::Loading {
            // A dashed line with a red head sweeping along it.
            let head = ((since / 1.2).fract()) as f32;
            for i in 0..BARS {
                let u = i as f32 / (BARS - 1) as f32;
                let near = (1.0 - (u - head).abs() * 8.0).clamp(0.0, 1.0);
                let c = mix(hex(YELLOW).fade(0.45), hex(RED), near);
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x0 + i as f32 * pitch - 2.0, cy - 0.8, 4.0, 1.6), 0.0),
                    Paint::solid(c),
                ));
            }
            return;
        }
        // Heights freeze at the stop and settle; the final text flattens them.
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let settle = self
            .finish_at
            .map_or(1.0, |f| 1.0 - 0.7 * ease_out((t - f) / 0.4));
        let flat = self
            .final_at
            .map_or(0.0, |f| ease_out((t - f) / 0.24));
        // A cancel drops the bars to dashes at once.
        let t_eff = self.cancel_at.map_or(t_eff, |c| t_eff.min(c));
        let settle = if self.cancel_at.is_some() { 0.0 } else { settle };
        let cancelled = self.mode == Mode::Cancelled;
        // Bars grow out of the centre as the popup opens.
        let open = if reduced() {
            1.0
        } else {
            ease_out((since - 0.15) / 0.3)
        };
        let half_bars = (BARS / 2) as f32;
        for i in 0..BARS {
            let u = i as f32 / (BARS - 1) as f32 * 2.0 - 1.0;
            // Distance from the centre in bars: the newest audio is in the middle.
            let d = (i as f32 - (BARS as f32 - 1.0) / 2.0).abs();
            let reach = (d / half_bars).min(1.0);
            if reach > open {
                continue;
            }
            let level = self.level_at(t_eff - d as f64 * BAR_SECS);
            let taper = (1.0 - reach * reach).max(0.0).powf(2.2);
            let norm = (level * 1.25).min(1.0) * taper * settle * (1.0 - flat);
            let half = norm * WAVE_HALF;
            let x = x0 + i as f32 * pitch;
            let mut c = bar_hue(u);
            if norm > 0.62 {
                c = if i % 3 == 0 { hex(PINK) } else { hex(RED) };
            }
            if let (Mode::Finishing, Some(f)) = (self.mode, self.progress) {
                if (u + 1.0) / 2.0 > f {
                    c = hex(GREY);
                }
            }
            if cancelled {
                c = mix(c, hex(RED), 0.8).fade(0.6);
            }
            if half < 2.0 {
                // Quiet: a short dash.
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x - 2.2, cy - 0.9, 4.4, 1.8), 0.0),
                    Paint::solid(c.fade(0.85 * (1.0 - flat))),
                ));
            } else {
                p.shape(ShapeInstance::rrect(
                    RRect::new(Rect::new(x - 1.9, cy - half, 3.8, 2.0 * half), 0.0),
                    Paint::solid(c),
                ));
            }
        }
        if flat > 0.0 {
            // One thin line that opens from the centre.
            let mid = x0 + span / 2.0;
            let half = span / 2.0 * flat;
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(mid - half, cy - 0.7, 2.0 * half, 1.4), 0.0),
                Paint::solid(hex(YELLOW)),
            ));
        }
    }
}

/// A bar's colour by its place, -1 at the left end to 1 at the right: red
/// tails, a yellow middle, magenta on the right.
fn bar_hue(u: f32) -> Color {
    let stops = [
        (-1.0, RED),
        (-0.62, ORANGE),
        (-0.38, YELLOW),
        (0.42, YELLOW),
        (0.66, ORANGE),
        (0.82, RED),
        (1.0, PINK),
    ];
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if u <= b {
            return mix(hex(ca), hex(cb), (u - a) / (b - a));
        }
    }
    hex(PINK)
}

/// The hazard slashes at the top right: they slide in on the open and drift
/// while the voice is up.
#[derive(Clone)]
struct Slashes {
    shown_at: f64,
    /// Drift until this time; still after it.
    drift_until: f64,
    red: bool,
}

impl Slashes {
    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let since = t - self.shown_at;
        let colour = if self.red { hex(RED) } else { hex(YELLOW) };
        let pitch = 12.5;
        let drift = if reduced() {
            0.0
        } else {
            ((t.min(self.drift_until) / 1.2).fract() as f32) * pitch
        };
        // Five slashes slide under a clip of four.
        for i in 0..5 {
            let k = if reduced() {
                1.0
            } else {
                ease_out((since - 0.15 - i as f64 * DRAW_STAGGER) / 0.3)
            };
            if k <= 0.0 {
                continue;
            }
            let x = rect.x + i as f32 * pitch - drift + (1.0 - k) * 40.0;
            let pts = [(x + 8.0, 0.0), (x + 15.0, 0.0), (x + 7.0, 13.0), (x, 13.0)];
            // Keep each slash inside the strip.
            let clip = |v: f32| v.clamp(rect.x, rect.x + rect.w);
            let pts: Vec<(f32, f32)> = pts.iter().map(|&(px, py)| (clip(px), py)).collect();
            p.fill(
                [&poly(&pts, Point::new(0.0, rect.y))],
                &StrokeStyle::solid(0.0, colour.fade(k)).glow(2.0),
            );
        }
    }
}

/// The small icon in the tag and the one in the status bar.
#[derive(Clone)]
struct Icons {
    mode: Mode,
    mode_at: f64,
    /// Ink for the tag icon, accent for the status icon.
    status: bool,
}

impl Icons {
    fn draw(&self, t: f64, rect: Rect, p: &mut Painter) {
        let c = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let r = rect.w.min(rect.h) / 2.0;
        let k = if reduced() {
            1.0
        } else {
            ease_out((t - self.mode_at) / 0.2)
        };
        let ink = if self.status { hex(YELLOW) } else { hex(INK) };
        match self.mode {
            Mode::Loading | Mode::Listening if !self.status => {
                p.shape(ShapeInstance::circle(c, r * 0.8, Paint::solid(hex(RED))));
            }
            Mode::Loading | Mode::Listening | Mode::Finishing if self.status => {
                // A microphone: capsule, cradle and stand.
                let style = StrokeStyle::solid(1.6, ink);
                p.shape(ShapeInstance::capsule(
                    Rect::new(c.x - r * 0.32, c.y - r, r * 0.64, r * 1.15),
                    Paint::solid(ink),
                ));
                p.stroke(
                    &make::arc(Point::new(c.x, c.y - 0.1 * r), r * 0.6, 0.0, std::f32::consts::PI),
                    &style,
                );
                p.stroke(
                    &Contour::polyline(
                        &[Point::new(c.x, c.y + 0.5 * r), Point::new(c.x, c.y + r)],
                        false,
                    ),
                    &style,
                );
                p.stroke(
                    &Contour::polyline(
                        &[
                            Point::new(c.x - 0.4 * r, c.y + r),
                            Point::new(c.x + 0.4 * r, c.y + r),
                        ],
                        false,
                    ),
                    &style,
                );
            }
            Mode::Finishing => {
                // Three segments turning once every 0.9 s.
                let turn = (t / 0.9).fract() as f32 * std::f32::consts::TAU;
                for i in 0..3 {
                    let start = turn + i as f32 * std::f32::consts::TAU / 3.0;
                    p.stroke(
                        &make::arc(c, r * 0.75, start, 1.4),
                        &StrokeStyle::solid(1.8, hex(RED)),
                    );
                }
            }
            Mode::Done => {
                let check = Outline::new(&icons::check(), 0.05)
                    .transform(&Affine::translate(-12.0, -12.0).then(&Affine::scale(r / 9.0)))
                    .transform(&Affine::translate(c.x, c.y));
                let style = StrokeStyle::solid(2.2, ink);
                for contour in &check.contours {
                    p.stroke(&contour.trim(0.0, contour.length() * k), &style);
                }
            }
            Mode::Cancelled => {
                let x = Outline::new(&icons::x(), 0.05)
                    .transform(&Affine::translate(-12.0, -12.0).then(&Affine::scale(r / 8.0)))
                    .transform(&Affine::translate(c.x, c.y));
                let ink = if self.status { hex(RED) } else { hex(PAPER) };
                let style = StrokeStyle::solid(2.2, ink);
                for contour in &x.contours {
                    p.stroke(&contour.trim(0.0, contour.length() * k), &style);
                }
            }
            _ => {}
        }
    }

    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Finishing => now + 1.0,
            _ => self.mode_at + 0.25,
        }
    }
}

impl Popup {
    fn cyber_mode(&self) -> Mode {
        let status = self.status();
        if self.extra.final_at.is_some() {
            Mode::Done
        } else if self.ended() {
            if self.extra.finish_at.is_some() {
                Mode::Done
            } else {
                Mode::Cancelled
            }
        } else if status.loading {
            Mode::Loading
        } else if status.finishing || self.extra.finish_at.is_some() {
            Mode::Finishing
        } else {
            Mode::Listening
        }
    }

    /// Brings the look's own state up to date after a message.
    pub(crate) fn sync_cyber(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Cyberpunk {
            return;
        }
        let mode = self.cyber_mode();
        let c = &mut self.cyber;
        if c.mode != Some(mode) {
            let old = c.mode.map_or("", Mode::tag);
            if old != mode.tag() {
                c.old_tag = old;
                c.tag_at = if c.mode.is_none() { self.shown_at } else { t };
            }
            if c.mode.is_some() {
                c.mode_at = t;
            }
            c.mode = Some(mode);
        }
        if self.lines != c.lines {
            c.old_lines = c.lines;
            c.lines = self.lines;
            c.lines_at = t;
        }
        // A word whose text changed, other than by growing, is a revision.
        // An edit is not one: no glitch burst and no roll while editing.
        let editing = self.edit_view.is_some();
        if !editing && self.extra.final_at.is_none() && t - c.revised_at > BURST_GAP {
            let bare = |w: &str| w.trim_start_matches('…').to_string();
            let revised = self.shown.iter().find(|w| {
                c.words.iter().any(|o| {
                    o.id == w.id && {
                        let (a, b) = (bare(&o.text), bare(&w.text));
                        a != b && !b.starts_with(&a)
                    }
                })
            });
            if let Some(w) = revised {
                c.revised_at = t;
                c.revised = Some(w.id);
            }
        }
        // Every changed word rolls its old text up and out.
        c.rolls.retain(|r| t < r.2 + ROLL_SECS);
        for w in self.shown.iter().filter(|_| !editing) {
            if let Some(o) = c.words.iter().find(|o| o.id == w.id && o.text != w.text) {
                c.rolls.push((w.id, o.text.clone(), t));
            }
        }
        c.words.clone_from(&self.shown);
    }

    /// Whether short ticks should run at `t`: the open, the typing, the
    /// growth and the bursts.
    fn cyber_busy(&self, t: f64) -> bool {
        let c = &self.cyber;
        let tag = c.mode.map_or("", Mode::tag);
        let typing = c.tag_at + TYPE_DELAY + typing_secs(c.old_tag, tag);
        !self.closing
            && (t < self.shown_at + 0.7
                || t < typing
                || t < c.lines_at + GROW_SECS
                || t < c.revised_at + BURST_SECS
                || c.rolls.iter().any(|r| t < r.2 + ROLL_SECS)
                || t < c.mode_at + 0.25)
    }

    /// Keeps the tick armed while the look needs it.
    pub(crate) fn arm_cyber_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.look.theme == OsdTheme::Cyberpunk
            && self.extra.tick.is_none()
            && self.cyber_busy(cx.t())
        {
            self.extra.tick = Some(cx.after(1.0 / TICK_HZ, Msg::Tick));
        }
    }

    /// The surface: the panel at its tallest and room round it for the slices.
    pub(crate) fn cyber_surface(look: &Look) -> Size {
        Size::new(
            (width(look) + 2.0 * MARGIN_X).ceil(),
            (tall_h() + 2.0 * MARGIN_Y).ceil(),
        )
    }

    fn tag_style(&self) -> TextStyle {
        TextStyle::new(fonts(&self.look.fonts).tag, TAG_PX, 20.0).tracking(0.5)
    }

    /// The tag's right end with `text` in it.
    fn tag_end(&self, text: &str) -> f32 {
        let w = self.measure.layout(text, &self.tag_style(), None).size.w;
        TAG_TEXT_X + w + TAG_PAD
    }

    pub(crate) fn cyber_view(&self) -> Element<Msg> {
        let l = &self.look;
        let size = Self::cyber_surface(l);
        let root = Element::column("root").size(size.w, size.h);
        if self.closing {
            return root;
        }
        root.child(self.cyber_panel(size))
    }

    fn cyber_panel(&self, size: Size) -> Element<Msg> {
        let l = &self.look;
        let c = &self.cyber;
        let t = self.extra.now;
        let mode = c.mode.unwrap_or(Mode::Loading);
        let tag = mode.tag();
        let status = self.status();
        let w = width(l);
        let progress = (mode == Mode::Finishing || mode == Mode::Loading)
            .then(|| self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32))
            .flatten()
            .filter(|_| self.shows_progress(&status));

        let mut bursts = vec![(c.revised_at, BURST_SECS, 0.2)];
        if c.mode_at.is_finite() {
            let strong = if mode == Mode::Cancelled { 0.8 } else { 0.4 };
            bursts.push((c.mode_at, 0.16, strong));
        }
        let deco = Deco {
            w,
            shown_at: self.shown_at,
            h0: panel_h(c.old_lines as f32),
            h1: panel_h(c.lines as f32),
            grow_at: c.lines_at,
            tag0: if c.old_tag.is_empty() {
                self.tag_end(tag)
            } else {
                self.tag_end(c.old_tag)
            },
            tag1: self.tag_end(tag),
            tag_at: c.tag_at,
            mode,
            mode_at: c.mode_at,
            bursts,
            progress,
            mark: c
                .revised
                .and_then(|id| self.cyber_word_box(id))
                .map(|b| (c.revised_at, b)),
        };
        // The panel's top moves down by what it lacks of its full height.
        let dy = tall_h() - deco.h(t);
        let sy = tall_h() - STATUS_BOTTOM - STATUS_H;
        let until = deco.until();
        let deco = at(
            Element::canvas("deco", until, move |t, rect, p| deco.draw(t, rect, p)),
            0.0,
            0.0,
        )
        .size(size.w, size.h)
        .declare_reach(DECO_REACH);

        // A burst also glitches the transcript and the waveform for a moment.
        // Not while editing: the text stays where it is laid out.
        let burst_on = !reduced()
            && self.edit_view.is_none()
            && (t - c.mode_at < 0.16 || t - self.shown_at < OPEN_GLITCH_SECS);

        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.97)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let mut panel = at(Element::layer("panel"), 0.0, 0.0)
            .size(size.w, size.h)
            .presence(Transition::fade(), exit)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12));
        if burst_on {
            panel = panel.glitch(0.35).chromatic(1.5).motion(Prop::Style, Motion::Snap);
        }
        let (ox, oy) = (MARGIN_X, MARGIN_Y);
        // Everything hung from the panel's top rides on `body`, which the
        // tick moves with the canvas's height, so it snaps rather than springs.
        let body = at(Element::column("body"), 0.0, dy)
            .size(size.w, size.h - dy)
            .motion(Prop::Position, Motion::Snap)
            .child(self.cyber_tag(mode, ox, oy))
            .child(self.cyber_slashes(mode, w, ox, oy))
            .child(
                at(
                    Element::label("audio", "// AUDIO")
                        .text_style(TextStyle::new(fonts(&self.look.fonts).tag, 10.0, 12.0).tracking(0.6))
                        .color(if mode == Mode::Cancelled {
                            hex(RED)
                        } else {
                            hex(YELLOW)
                        }),
                    ox + w - 137.0,
                    oy + 15.0,
                )
                .presence(Transition::fade(), Transition::fade()),
            )
            .child(self.cyber_wave(mode, progress, ox, oy));
        // The transcript hangs from the panel's bottom: it takes the place
        // of the grown panel at once, so the newest line stays put and the
        // older ones step up a line while the panel's top follows (RD-09).
        // It is still cut at the status bar's top.
        let text_dy = tall_h() - panel_h(c.lines as f32);
        let text = at(Element::layer("textclip"), 0.0, 0.0)
            .size(size.w, oy + sy - 2.0)
            .clip()
            .child(
                at(Element::column("textmove"), 0.0, text_dy)
                    .size(size.w, size.h - text_dy)
                    .motion(Prop::Position, Motion::Snap)
                    .child(self.cyber_text(mode, ox, oy)),
            );
        panel
            .child(deco)
            .child(body)
            .child(text)
            .child(self.cyber_status(mode, &status, ox, oy + sy))
            .child(self.cyber_count(mode, ox + w - 84.0, oy + sy + 17.0))
    }

    /// The tag's icon and its typed text with the block cursor.
    fn cyber_tag(&self, mode: Mode, ox: f32, oy: f32) -> Element<Msg> {
        let c = &self.cyber;
        let t = self.extra.now;
        let delay = if c.old_tag.is_empty() { TYPE_DELAY } else { 0.0 };
        let since = t - c.tag_at - delay;
        let (text, lit) = if mode == Mode::Cancelled {
            // The popup closes within moments of a cancel: no time to type.
            (mode.tag().to_string(), false)
        } else {
            typed(c.old_tag, mode.tag(), since)
        };
        let ink = if mode == Mode::Cancelled {
            hex(PAPER)
        } else {
            hex(INK)
        };
        let icons = Icons {
            mode,
            mode_at: c.mode_at,
            status: false,
        };
        let until = icons.until(t);
        let mut tag = Element::column("tag")
            .size(260.0, 30.0)
            .child(at(
                Element::canvas("icon", until, move |t, rect, p| icons.draw(t, rect, p))
                    .size(11.0, 11.0),
                12.0,
                8.0,
            ))
            .child(at(
                Element::label("text", text.clone())
                    .text_style(self.tag_style())
                    .color(ink),
                TAG_TEXT_X - 35.0,
                4.0,
            ));
        if lit {
            let w = self.measure.layout(&text, &self.tag_style(), None).size.w;
            tag = tag.child(at(
                Element::shape("cursor", ShapeKind::RRect)
                    .size(7.0, 14.0)
                    .fill(ink)
                    .motion(Prop::Position, Motion::Snap),
                TAG_TEXT_X - 35.0 + w + 3.0,
                7.0,
            ));
        }
        at(tag, ox + 35.0, oy + 34.0)
    }

    fn cyber_slashes(&self, mode: Mode, w: f32, ox: f32, oy: f32) -> Element<Msg> {
        let now = self.extra.now;
        // Drift while the voice is up; rest when quiet or stopped.
        let loud = self.levels.iter().rev().take(20).any(|l| *l > 0.08);
        let drift_until = if mode == Mode::Listening && loud {
            now + 0.25
        } else {
            now
        };
        let slashes = Slashes {
            shown_at: self.shown_at,
            drift_until,
            red: mode == Mode::Cancelled,
        };
        let until = drift_until.max(self.shown_at + 0.7);
        at(
            Element::canvas("slashes", until, move |t, rect, p| slashes.draw(t, rect, p))
                .size(50.0, 13.0),
            ox + w - 86.0,
            oy + 10.0,
        )
    }

    fn cyber_wave(&self, mode: Mode, progress: Option<f32>, ox: f32, oy: f32) -> Element<Msg> {
        let l = &self.look;
        let bars = Bars {
            levels: self.wave.clone(),
            newest_at: self.newest_at,
            shown_at: self.shown_at,
            mode,
            finish_at: self.extra.finish_at,
            final_at: self.extra.final_at,
            progress,
            cancel_at: (mode == Mode::Cancelled).then_some(self.cyber.mode_at),
        };
        let until = bars.until(self.extra.now);
        let span = width(l) - (DESIGN_W - WAVE_X1) - WAVE_X0;
        let box_h = 2.0 * WAVE_HALF + 8.0;
        at(Element::layer("wavelights"), ox + WAVE_X0 - 4.0, oy + WAVE_Y - box_h / 2.0)
            .size(span + 8.0, box_h)
            .bloom(0.35, 5.0)
            .child(at(
                Element::canvas("bars", until, move |t, rect, p| bars.draw(t, rect, p))
                    .size(span, box_h),
                4.0,
                0.0,
            ))
    }

    fn cyber_text(&self, mode: Mode, ox: f32, oy: f32) -> Element<Msg> {
        let l = &self.look;
        let cancelled = mode == Mode::Cancelled;
        let colour = if cancelled {
            hex(PAPER).fade(0.45)
        } else {
            hex(PAPER)
        };
        // New words rise into place. A revised word's replacement is there
        // at once, at full ink on the reading line, while its old text rolls
        // up and out above it (`cyber_rolls`).
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .text_style(l.text_style())
            .color(colour)
            .motion(Prop::TextColor, Spring::critical_to_98(0.2))
            .motion(Prop::Opacity, Spring::critical_to_98(0.10))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(Transition::fade().offset(0.0, 6.0)))
            .word_revise(Revise::Swap)
            .text_halo(Color::WHITE.fade(if cancelled { 0.0 } else { 0.12 }), 3.0);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            colour,
            hex(YELLOW),
        );
        // MO-50 as cyberpunk's cancel pick (MO-75): the words wind back
        // into a yellow caret over the hidden text, in place of the red strike.
        let rewinding = self.rewinding();
        let words = words.rewind_hide(rewinding);
        let mut text = at(
            Element::column("text")
                .width(l.text_width())
                .height(self.lines as f32 * TEXT_LINE),
            ox + TEXT_X,
            oy + TEXT_TOP,
        )
        .child(words)
        // MO-73: a caret on a foot that onsets split.
        .children(self.joint_ticks_element(
            "cyber",
            &self.shown,
            &l.text_style(),
            0.0,
            0.0,
            l.text_width(),
            hex(YELLOW),
            true,
        ))
        .children(self.cyber_rewind());
        if !rewinding {
            text = text.children(self.cyber_rolls(colour));
        }
        if cancelled && !rewinding {
            // A red strike through each line, drawn on.
            for (i, (y, w)) in self.cyber_line_extents().into_iter().enumerate() {
                let line = Arc::new(Outline {
                    contours: vec![make::underline(0.0, w, 0.5)],
                });
                text = text.child(at(
                    Element::stroke_path(
                        format!("strike{i}"),
                        line,
                        Rect::new(0.0, 0.0, w, 1.0),
                        StrokeStyle::solid(2.0, hex(RED)),
                    )
                    .size(w, 1.0)
                    .motion(Prop::Trim, Spring::critical_to_98(0.15))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    y,
                ));
            }
        }
        self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), colour)
    }

    /// Each revised word's old text, rolling up out of its slot and fading,
    /// its letters spreading to the new word's width. In the text column's
    /// space.
    fn cyber_rolls(&self, colour: Color) -> Vec<Element<Msg>> {
        let t = self.extra.now;
        let style = self.look.text_style();
        let travel = if reduced() {
            0.0
        } else {
            ROLL_WORD_EMS * style.size
        };
        let mut out = Vec::new();
        for (i, (id, old, at_t)) in self.cyber.rolls.iter().enumerate() {
            let k = ease_out((t - at_t) / ROLL_SECS);
            if k >= 1.0 {
                continue;
            }
            let Some(b) = self.cyber_word_box(*id) else {
                continue;
            };
            let new = self
                .shown
                .iter()
                .find(|w| w.id == *id)
                .map_or("", |w| w.text.as_str());
            let width = |s: &str| self.measure.layout(s, &style, None).size.w;
            let gaps = old.chars().count().saturating_sub(1).max(1) as f32;
            let spread = (width(new) - width(old)) / gaps * k;
            let roll = Element::label(format!("roll{i}"), old.clone())
                .text_style(style.tracking(style.tracking + spread))
                .color(colour)
                .opacity(1.0 - k)
                .motion(Prop::Position, Motion::Snap)
                .motion(Prop::Opacity, Motion::Snap);
            out.push(at(roll, b.x - TEXT_X, b.y - TEXT_TOP - travel * k));
        }
        out
    }

    /// MO-50: a cancel takes the words back into the caret.
    fn cyber_rewind(&self) -> Option<Element<Msg>> {
        if !self.rewinding() {
            return None;
        }
        let l = &self.look;
        let ink = endings::Ink {
            text: hex(PAPER).fade(0.45),
            mark: hex(YELLOW),
            faint: hex(RED),
        };
        let (style, w) = (l.text_style(), l.text_width());
        let h = self.lines.max(1) as f32 * TEXT_LINE;
        self.rewind_mark("rewind", self.lay_words(style, w), ink, Size::new(w, h))
    }

    /// The box of word `id` on screen, in panel space.
    fn cyber_word_box(&self, id: WordId) -> Option<Rect> {
        let l = &self.look;
        let mut start = 0usize;
        let mut range = None;
        for w in &self.shown {
            if w.id == id {
                range = Some(start as u32..(start + w.text.len()) as u32);
            }
            start += w.text.len() + 1;
        }
        let range = range?;
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let style = l.text_style();
        let shaped = self.measure.layout(&text, &style, Some(l.text_width()));
        let xs: Vec<(f32, f32)> = shaped
            .glyphs
            .iter()
            .filter(|g| range.contains(&g.cluster))
            .map(|g| (g.pos.x, g.pos.y))
            .collect();
        let (x0, y) = *xs.first()?;
        let x1 = xs.iter().map(|p| p.0).fold(x0, f32::max) + style.size * 0.55;
        let row = (y / style.line_height).floor();
        Some(Rect::new(
            TEXT_X + x0,
            TEXT_TOP + row * style.line_height,
            x1 - x0,
            style.line_height,
        ))
    }

    /// Each transcript line's middle and its width, from the shaped text.
    fn cyber_line_extents(&self) -> Vec<(f32, f32)> {
        let l = &self.look;
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let style = l.text_style();
        let shaped = self.measure.layout(&text, &style, Some(l.text_width()));
        let mut lines: Vec<(f32, f32)> = Vec::new();
        for g in &shaped.glyphs {
            let row = (g.pos.y / style.line_height).floor();
            let y = row * style.line_height + style.line_height * 0.55;
            let right = g.pos.x + style.size * 0.5;
            match lines.iter_mut().find(|(ly, _)| (*ly - y).abs() < 1.0) {
                Some(line) => line.1 = line.1.max(right),
                None => lines.push((y, right)),
            }
        }
        lines
    }

    fn cyber_status(&self, mode: Mode, status: &StatusLine, ox: f32, y: f32) -> Element<Msg> {
        let l = &self.look;
        let f = fonts(&l.fonts);
        let (label, detail): (String, String) = match mode {
            Mode::Done => {
                let app = self
                    .files
                    .live
                    .as_ref()
                    .and_then(|d| d.target.as_ref())
                    .map(|t| t.app.clone());
                (
                    "Done".into(),
                    app.map_or("text sent".into(), |a| format!("sent to {a}")),
                )
            }
            Mode::Cancelled => ("Cancelled".into(), "nothing was typed".into()),
            Mode::Listening => ("Listening".into(), status.detail.clone()),
            _ => (status.state.clone(), status.detail.clone()),
        };
        // The bar draws its own separator.
        let detail = detail.trim_start_matches(['·', ' ']).to_string();
        let accent = if mode == Mode::Cancelled {
            hex(RED)
        } else {
            hex(YELLOW)
        };
        let icons = Icons {
            mode,
            mode_at: self.cyber.mode_at,
            status: true,
        };
        let until = icons.until(self.extra.now);
        let room = width(l) - (DESIGN_W - 440.0) - 99.0;
        let mut row = Element::row("status")
            .width(room)
            .height(STATUS_H)
            .align_items(AlignItems::CENTER)
            .child(
                // The tag types the change of state; the label swaps in place.
                Element::label("state", label.as_str())
                    .text_style(TextStyle::new(f.tag, 16.0, 19.0).tracking(0.2))
                    .color(accent),
            );
        if !detail.is_empty() {
            row = row
                .child(
                    Element::label("sep", "·")
                        .text_style(TextStyle::new(f.body, 13.0, 18.0))
                        .color(hex(PAPER))
                        .margin_edges(0.0, 9.0, 0.0, 9.0),
                )
                .child(
                    Element::label("detail", detail.as_str())
                        .text_style(TextStyle::new(f.body, 13.5, 18.0).clamp_lines(1))
                        .color(hex(PAPER).fade(0.92))
                        .shrink(1.0)
                        .min_width(0.0),
                );
        }
        Element::column("statusbox")
            .size(width(l), STATUS_H)
            .layout(|s| {
                s.position = taffy::Position::Absolute;
                s.inset = taffy::Rect {
                    left: length(ox),
                    top: length(y),
                    right: auto(),
                    bottom: auto(),
                };
            })
            .child(at(
                Element::canvas("icon", until, move |t, rect, p| icons.draw(t, rect, p))
                    .size(18.0, 18.0),
                63.0,
                (STATUS_H - 18.0) / 2.0,
            ))
            .child(at(row, 99.0, 0.0))
    }

    /// "// 01": the transcript's line count, rolling; "// OK" once done.
    fn cyber_count(&self, mode: Mode, x: f32, y: f32) -> Element<Msg> {
        let style = TextStyle::new(fonts(&self.look.fonts).mono, 11.5, 14.0);
        let value = match mode {
            Mode::Done => "OK".to_string(),
            Mode::Cancelled => "XX".to_string(),
            _ => format!("{:02}", self.lines.min(99)),
        };
        at(
            Element::row("count")
                .gap(5.0)
                .child(Element::label("mark", "//").text_style(style).color(hex(RED)))
                .child(digit_roll(
                    "digits",
                    &value,
                    style,
                    hex(RED),
                    Spring::critical_to_98(0.25),
                )),
            x,
            y,
        )
    }
}

/// Cyberpunk's edit adapter (plan section 4.5): the yellow caret of the
/// joint ticks is the edit caret, and the ticks, which follow the sound,
/// are off.
pub(crate) struct CyberpunkSite;

impl edit_site::EditSite for CyberpunkSite {
    fn caret(&self, _look: &Look) -> edit_site::CaretStyle {
        edit_site::CaretStyle {
            caret: vo::scene::flow::Caret {
                color: Some(hex(YELLOW)),
                ..vo::scene::flow::Caret::default()
            },
            selection: Color::hex(YELLOW, 0.35),
            mark: hex(YELLOW),
        }
    }

    fn quiet(&self) -> &'static [&'static str] {
        &[sounding::ONSET_JOINT]
    }
}

#[cfg(test)]
mod edit_tests {
    use super::super::swiss_look::edit_check::{envelope, Rig};
    use super::{hex, sounding, YELLOW};
    use voxtype::osd::config::OsdTheme;

    /// M7.L4: Cyberpunk edits in its own layout, its yellow caret as the edit caret, its glitch bursts, rolls and joint ticks off. With the caret on the
    /// newest, a middle and the top line, at both widths: the caret on its
    /// glyph within 1 px in every frame, the popup's size unchanged, and the
    /// golden `tests/fixtures/osd-vo/edit-cyberpunk.txt`.
    #[test]
    fn cyberpunk_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Cyberpunk, "cyberpunk");
    }

    /// The edit caret is the look's yellow; with the joint on, its ticks
    /// and their caret are off while editing, so one caret shows.
    #[test]
    fn cyberpunk_edits_with_one_yellow_caret() {
        for joint in [false, true] {
            let mut r = Rig::new(OsdTheme::Cyberpunk, false);
            if joint {
                let mut on = r.p.look.features.to_vec();
                on.push(sounding::ONSET_JOINT.into());
                r.p.look.features = on.into();
            }
            r.dictate(160);
            r.enter(1.0);
            assert_eq!(r.carets(hex(YELLOW)), 1, "joint {joint}");
        }
    }

    /// M7.L4: the edit moment keeps the reading-line envelope (R35).
    #[test]
    fn cyberpunk_edits_within_the_envelope() {
        envelope(OsdTheme::Cyberpunk, "cyberpunk");
    }
}
