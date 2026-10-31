//! The `biolume` look of the vo popup, after concept 15, bioluminescent: a
//! jellyfish. Its bell is a dark translucent dome with a lumpy rim that
//! glows blue to violet, and glowing tendrils hang below it. In the bell a
//! mirrored bar waveform stands at the top, the words sit under it in pale
//! cyan type with a soft glow, centred and growing downward, and a centred
//! status row runs along the bell's lower edge.
//!
//! While the final text is made the waveform lowers; when the text lands
//! the bars settle and the status turns green. A cancel greys the bell and
//! the tendrils and dims the words. While the popup is open and listening
//! the bell's glow breathes and the tendrils sway; under reduced motion
//! nothing in them moves.

use super::glyph_paths::PathsText;
use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, Rect};
use vo::scene::path::{make, Contour, StrokeStyle};
use vo::scene::{Loop, LoopProp};

/// Colours sampled from the redesign.
const DEEP: u32 = 0x040a26;
const BODY: u32 = 0x0a1650;
const RIM: u32 = 0x3d7bff;
const CYAN: u32 = 0x38d8ff;
const BLUE: u32 = 0x3a62ff;
const VIOLET: u32 = 0x9a5cff;
const TEXT: u32 = 0xd6f2ff;
const HINT: u32 = 0xb4cdf2;
const DIM: u32 = 0x66708c;
const GREEN: u32 = 0x39d98a;

/// The jellyfish in the redesign's units at `width_px = 600`; the look
/// scales with the popup's width.
const DESIGN_W: f32 = 600.0;
/// The bell: a rounded dome, its middle, half width and half height, and
/// how square its sides are (a superellipse's power).
const BELL_X: f32 = 300.0;
const BELL_Y: f32 = 216.0;
const BELL_RX: f32 = 276.0;
const BELL_RY: f32 = 196.0;
const BELL_POWER: f32 = 2.6;
/// Points round the bell's rim.
const RIM_POINTS: usize = 120;
/// The waveform at the top of the bell.
const WAVE_Y: f32 = 96.0;
const WAVE_X0: f32 = 165.0;
const WAVE_X1: f32 = 435.0;
const WAVE_HALF: f32 = 30.0;
const BARS: usize = 41;
/// Seconds of audio between neighbouring bars, out from the middle.
const BAR_SECS: f64 = 0.03;
/// Levels under this are room tone and draw flat.
const WAVE_GATE: f32 = 0.04;
/// The words: centred in this width from this top, growing downward.
const TEXT_W: f32 = 430.0;
const TEXT_PX: f32 = 29.0;
const TEXT_LINE: f32 = 38.0;
const TEXT_TOP: f32 = 146.0;
const CLIP_TOP: f32 = 134.0;
const MAX_LINES: u32 = 4;
/// The status row's middle, inside the bell's lower edge.
const STATUS_Y: f32 = 346.0;
const STATUS_H: f32 = 20.0;
const STATUS_W: f32 = 380.0;
const STATUS_PX: f32 = 14.0;
/// The tendrils: how many, where they start under the bell and how long
/// they hang.
const TENDRILS: u32 = 22;
const ARMS: u32 = 5;
const TENDRIL_MIN: f32 = 150.0;
const TENDRIL_MAX: f32 = 230.0;
/// The surface: the bell's top room and the tendrils' room below.
const SURFACE_H: f32 = 660.0;
const SETTLE_SECS: f64 = 0.35;
/// Idle (RD-02): the pace of the idle motion; the glow's lowest opacity and
/// seconds per breath; how far a tendril's tip sways, how often, and how
/// far the bend lags from the bell to the tip, in radians.
const IDLE_FPS: f64 = 8.0;
/// The frame pace while the popup records room tone (RD-11): the tendrils
/// redraw at the idle pace, at most 0.8 px a step.
pub(crate) const ROOM_TONE_FPS: f64 = IDLE_FPS;
const GLOW_LOW: f64 = 0.8;
const GLOW_PERIOD: f64 = 4.0;
const SWAY_PX: f32 = 4.0;
const SWAY_HZ: f64 = 0.25;
const SWAY_LAG: f32 = 2.4;

// The waveform, the words and the status sit inside the bell, and the
// longest tendril inside the surface.
const _: () = assert!(BELL_Y - BELL_RY > 8.0);
const _: () = assert!(WAVE_Y - WAVE_HALF > BELL_Y - BELL_RY + 20.0);
const _: () = assert!(CLIP_TOP > WAVE_Y + WAVE_HALF && CLIP_TOP < TEXT_TOP);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < STATUS_Y - STATUS_H);
const _: () = assert!(STATUS_Y + STATUS_H < BELL_Y + BELL_RY - 12.0);
const _: () = assert!(BELL_Y + BELL_RY + TENDRIL_MAX < SURFACE_H);

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn face(look: &Look) -> u32 {
    look.fonts
        .system("Fira Sans")
        .or_else(|| look.fonts.system("Google Sans Flex"))
        .or_else(|| look.fonts.system("Inter"))
        .unwrap_or(SANS)
}

/// The status font: Inter has the arrow a correction's detail shows.
fn ui(look: &Look) -> u32 {
    look.fonts.system("Inter").unwrap_or(SANS)
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = scale(look);
    TextStyle::new(face(look), TEXT_PX * k, TEXT_LINE * k)
        .tracking(-0.2 * k)
        .centered()
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    Size::new((DESIGN_W * k).ceil(), (SURFACE_H * k).ceil())
}

/// The middle of the bell: the centre anchor puts it on the reading line,
/// and the tendrils hang below.
pub(crate) fn panel_middle(look: &Look) -> f32 {
    BELL_Y * scale(look)
}

/// The middle of the newest line below the surface's top. The words grow
/// downward from the top of the text box.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (TEXT_TOP + (lines.clamp(1, MAX_LINES) as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// How far a change of state has gone, 0 to 1.
fn change(t: f64, at: f64, still: bool) -> f32 {
    if still {
        1.0
    } else {
        ease_out((t - at) / SETTLE_SECS)
    }
}

/// A number from 0 to 1 that depends only on `n`.
fn noise(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// The bell's rim at angle `a` (0 to the right, a quarter turn down), in
/// design units: a superellipse with faint lumps, scalloped below.
fn rim_at(a: f32) -> Point {
    let (s, c) = a.sin_cos();
    let p = 2.0 / BELL_POWER;
    // Faint lumps over the dome, and scallops along its lower edge.
    let low = s.max(0.0).powi(2);
    let lumps = 1.0 + 0.006 * (9.0 * a + 0.5).sin() + 0.03 * low * (16.0 * a + 0.8).sin();
    Point::new(
        BELL_X + BELL_RX * lumps * c.signum() * c.abs().powf(p),
        BELL_Y + BELL_RY * lumps * s.signum() * s.abs().powf(p),
    )
}

/// The rim's lowest point below `x`, in design units.
fn bell_bottom(x: f32) -> f32 {
    let u = ((x - BELL_X) / BELL_RX).clamp(-1.0, 1.0);
    BELL_Y + BELL_RY * (1.0 - u.abs().powf(BELL_POWER)).powf(1.0 / BELL_POWER)
}

/// One layer of the jellyfish, so the idle pulse runs on the glow alone and
/// the sway redraws only the tendrils.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Tendrils,
    Body,
    Glow,
    Edge,
}

/// The bell and its tendrils, in the surface's space.
#[derive(Clone, Copy)]
struct Jelly {
    k: f32,
    mode: Mode,
    mode_at: f64,
    still: bool,
    part: Part,
    /// The tendrils sway: the popup is open and listening or loading.
    sway: bool,
}

impl Jelly {
    fn until(&self) -> f64 {
        if self.part == Part::Tendrils && self.sway {
            return f64::INFINITY;
        }
        self.mode_at + SETTLE_SECS + 0.1
    }

    /// The sway's clock at `t`, in `IDLE_FPS` steps. It stops where it was
    /// when the listening ends, and stands at 0 under reduced motion.
    fn sway_t(&self, t: f64) -> f64 {
        if self.still {
            return 0.0;
        }
        let t = if self.sway { t } else { t.min(self.mode_at) };
        (t * IDLE_FPS).floor() / IDLE_FPS
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let c = change(t, self.mode_at, self.still);
        let (grey, power) = match self.mode {
            Mode::Cancelled => (c, 1.0 - 0.5 * c),
            Mode::Loading => (0.0, 0.6),
            _ => (0.0, 1.0),
        };
        let tint = |h: u32| mix(hex(h), hex(DIM), grey);
        let pt = |q: Point| Point::new(r.x + q.x * k, r.y + q.y * k);
        if self.part == Part::Tendrils {
            self.tendrils(self.sway_t(t), r, p, &tint, power);
            return;
        }
        // The bell: a dark translucent body, lighter towards the rim.
        let rim: Vec<Point> = (0..RIM_POINTS)
            // From the bottom, where the scallops hide the strokes' join.
            .map(|i| {
                let a = i as f32 / RIM_POINTS as f32 * std::f32::consts::TAU;
                pt(rim_at(a + std::f32::consts::FRAC_PI_2))
            })
            .collect();
        let bell = Contour::polyline(&rim, true);
        if self.part == Part::Body {
            let (top, bottom) = (
                pt(Point::new(BELL_X, BELL_Y - BELL_RY)),
                pt(Point::new(BELL_X, BELL_Y + BELL_RY)),
            );
            p.fill(
                [&bell],
                &StrokeStyle::solid(1.0, hex(BODY)).linear(
                    top,
                    mix(hex(BODY), hex(DEEP), grey).fade(0.88),
                    bottom,
                    hex(DEEP).fade(0.92),
                ),
            );
            return;
        }
        // The rim: light inside it, a wide soft glow, a bright edge and a
        // faint inner ring.
        let middle = pt(Point::new(BELL_X, BELL_Y));
        let inside = |f: f32| -> Vec<Point> {
            rim.iter()
                .map(|q| {
                    Point::new(
                        middle.x + (q.x - middle.x) * f,
                        middle.y + (q.y - middle.y) * f,
                    )
                })
                .collect()
        };
        if self.part == Part::Glow {
            p.stroke(
                &Contour::polyline(&inside(0.9), true),
                &StrokeStyle::solid(26.0 * k, tint(BLUE).fade(0.1 * power)).glow(22.0 * k),
            );
            p.stroke(
                &bell,
                &StrokeStyle::solid(7.0 * k, tint(RIM).fade(0.3 * power)).glow(18.0 * k),
            );
            return;
        }
        p.stroke(
            &bell,
            &StrokeStyle::solid(2.0 * k, tint(CYAN))
                .conic(
                    middle,
                    -std::f32::consts::FRAC_PI_2,
                    tint(CYAN).fade(power),
                    tint(VIOLET).fade(power),
                )
                .glow(6.0 * k),
        );
        let inner: Vec<Point> = rim
            .iter()
            .map(|q| {
                Point::new(
                    middle.x + (q.x - middle.x) * 0.95,
                    middle.y + (q.y - middle.y) * 0.94,
                )
            })
            .collect();
        p.stroke(
            &Contour::polyline(&inner, true),
            &StrokeStyle::solid(1.0 * k, tint(BLUE).fade(0.35 * power)),
        );
        // Specks of light along the rim.
        for i in 0..48u32 {
            let q = pt(rim_at(noise(i) * std::f32::consts::TAU));
            let a = (0.35 + 0.65 * noise(i + 100)) * power;
            p.shape(ShapeInstance::circle(
                q,
                1.2 * k,
                Paint::glow(tint(CYAN).fade(0.6 * a), 3.0 * k, 0.5),
            ));
            p.shape(ShapeInstance::circle(
                q,
                0.8 * k,
                Paint::solid(hex(TEXT).fade(a)),
            ));
        }
    }

    /// The tendrils under the bell: thin glowing strands that wave down
    /// from its lower edge, and a few wider, fainter arms in the middle.
    /// At `sway`, the sway's clock, each strand bends `SWAY_PX` at its tip
    /// at `SWAY_HZ`, the bend running down it from the bell.
    fn tendrils(
        &self,
        sway: f64,
        r: Rect,
        p: &mut Painter,
        tint: &dyn Fn(u32) -> Color,
        power: f32,
    ) {
        let k = self.k;
        let strand =
            |i: u32, x0: f32, len: f32, amp: f32, width: f32, alpha: f32, p: &mut Painter| {
                let (f, phase) = (
                    1.1 + 0.7 * noise(i + 7),
                    std::f32::consts::TAU * noise(i + 13),
                );
                let out = (x0 - BELL_X) / BELL_RX;
                let y0 = bell_bottom(x0) - 10.0;
                let n = 24;
                let beat = std::f64::consts::TAU * (SWAY_HZ * sway + noise(i + 17) as f64);
                let pts: Vec<Point> = (0..=n)
                    .map(|j| {
                        let s = j as f32 / n as f32;
                        let bend = SWAY_PX * s * (beat as f32 - SWAY_LAG * s).sin();
                        let x = x0
                            + bend
                            + out * 60.0 * s
                            + (3.0 + amp * s) * (std::f32::consts::TAU * f * s + phase).sin()
                            + 0.4
                                * amp
                                * s
                                * (std::f32::consts::TAU * 2.3 * f * s + 2.0 * phase).sin();
                        Point::new(r.x + x * k, r.y + (y0 + len * s) * k)
                    })
                    .collect();
                let (a, b) = (pts[0], pts[n]);
                let strand = Contour::polyline(&pts, false);
                p.stroke(
                    &strand,
                    &StrokeStyle::solid(width * k, tint(BLUE))
                        .linear(a, tint(BLUE).fade(alpha * power), b, tint(VIOLET).fade(0.0))
                        .glow(5.0 * k),
                );
                // A few specks along it.
                for m in 0..3u32 {
                    let s = 0.2 + 0.6 * noise(i * 5 + m);
                    let q = pts[(s * n as f32) as usize];
                    p.shape(ShapeInstance::circle(
                        q,
                        1.0 * k,
                        Paint::solid(tint(CYAN).fade(0.7 * alpha * power * (1.0 - s))),
                    ));
                }
            };
        for i in 0..TENDRILS {
            let u = i as f32 / (TENDRILS - 1) as f32;
            let x0 = BELL_X + (u - 0.5) * 2.0 * BELL_RX * 0.82 + 8.0 * (noise(i + 3) - 0.5);
            let len = TENDRIL_MIN
                + (TENDRIL_MAX - TENDRIL_MIN)
                    * (0.3 + 0.7 * noise(i + 31))
                    * (1.0 - 0.4 * (2.0 * u - 1.0).abs());
            let (amp, width, alpha) = (
                10.0 + 22.0 * noise(i + 41),
                1.1 + 0.9 * noise(i + 53),
                0.5 + 0.45 * noise(i + 67),
            );
            strand(i, x0, len, amp, width, alpha, p);
        }
        for i in 0..ARMS {
            let u = i as f32 / (ARMS - 1) as f32;
            let x0 = BELL_X + (u - 0.5) * 120.0;
            strand(100 + i, x0, TENDRIL_MAX * 0.9, 22.0, 5.0, 0.3, p);
        }
    }
}

/// The waveform at the top of the bell: bars mirrored out from the middle,
/// the newest audio in the middle, cyan to blue to violet.
#[derive(Clone)]
struct Wave {
    k: f32,
    levels: Levels,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    still: bool,
}

impl Wave {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => f64::INFINITY,
            _ => self.mode_at.max(now) + SETTLE_SECS + 0.1,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let cy = r.y + r.h / 2.0;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let c = change(t, self.mode_at, self.still);
        let (height, grey) = match self.mode {
            Mode::Loading => (0.0, 0.4),
            Mode::Finishing => (1.0 - 0.6 * c, 0.0),
            Mode::Done => (0.4 * (1.0 - c), 0.0),
            Mode::Cancelled => (1.0 - c, c),
            Mode::Listening => (1.0, 0.0),
        };
        let mid = BARS / 2;
        let pitch = r.w / (BARS - 1) as f32;
        for i in 0..BARS {
            let d = i.abs_diff(mid);
            let reach = d as f32 / mid as f32;
            let at = t_eff - d as f64 * BAR_SECS;
            // Room tone stays flat, so a silence holds still.
            let raw = self.levels.at(at);
            let raw = if raw < WAVE_GATE { 0.0 } else { raw };
            let jitter = 0.5 + 0.5 * noise((at / BAR_SECS).round() as i64 as u32);
            let taper = (1.0 - reach.powf(1.3)).max(0.0);
            let level = (raw * 2.2).min(1.0) * jitter * taper * height;
            let half = (level * WAVE_HALF * k).max(1.0 * k);
            let x = r.x + i as f32 * pitch;
            let u = i as f32 / (BARS - 1) as f32;
            let col = if u < 0.5 {
                mix(hex(CYAN), hex(BLUE), 2.0 * u)
            } else {
                mix(hex(BLUE), hex(VIOLET), 2.0 * u - 1.0)
            };
            let col = mix(col, hex(DIM), grey).fade(1.0 - 0.35 * reach);
            if half <= 1.2 * k {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.1 * k,
                    Paint::solid(col),
                ));
                continue;
            }
            let bar = Rect::new(x - 1.3 * k, cy - half, 2.6 * k, 2.0 * half);
            p.shape(ShapeInstance::capsule(
                bar,
                Paint::glow(col.fade(0.5), 5.0 * k, 0.4),
            ));
            p.shape(ShapeInstance::capsule(bar, Paint::solid(col)));
        }
    }
}

/// The look's transcript for the outline features.
fn biolume_paths(look: &Look) -> PathsText {
    PathsText {
        style: look.text_style(),
        width: look.text_width(),
        line: TEXT_LINE * scale(look),
        ink: hex(TEXT),
        nib: hex(CYAN),
    }
}

impl Popup {
    pub(crate) fn biolume_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.biolume_jelly())
    }

    fn biolume_jelly(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let size = surface(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let still = self.still();
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let idle = matches!(mode, Mode::Listening | Mode::Loading) && !still;
        let jelly = |part| Jelly {
            k,
            mode,
            mode_at,
            still,
            part,
            sway: idle,
        };
        let layer = |name: &'static str, part| {
            let j = jelly(part);
            at(
                Element::canvas(name, j.until(), move |t, r, p| j.draw(t, r, p))
                    .size(size.w, size.h),
                0.0,
                0.0,
            )
        };
        // The glow breathes from full to `GLOW_LOW` and back.
        let glow = if idle {
            layer("glow", Part::Glow).ambient(
                LoopProp::Opacity,
                Loop::pulse(GLOW_LOW - 1.0, GLOW_PERIOD)
                    .fps(IDLE_FPS)
                    .settle(Spring::SNAP),
            )
        } else {
            layer("glow", Part::Glow)
        };
        let wave = Wave {
            k,
            levels: Levels {
                levels: self.exhale_levels(&self.kit_levels().levels),
                newest_at: self.newest_at,
            },
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
            still,
        };
        let wave_until = wave.until(self.extra.now);
        let lines = self.lines.clamp(1, MAX_LINES);
        let text_x = (DESIGN_W - TEXT_W) / 2.0 * k;
        Element::layer("biolume")
            .size(size.w, size.h)
            .presence(Transition::fade().offset(0.0, 8.0), exit)
            .child(layer("tendrils", Part::Tendrils))
            .child(layer("body", Part::Body))
            .child(glow)
            .child(layer("edge", Part::Edge))
            .child(at(
                Element::canvas("wave", wave_until, move |t, r, p| wave.draw(t, r, p))
                    .size((WAVE_X1 - WAVE_X0) * k, 2.0 * WAVE_HALF * k),
                WAVE_X0 * k,
                (WAVE_Y - WAVE_HALF) * k,
            ))
            .children(self.limit_line(
                Point::new(text_x, TEXT_TOP * k),
                size,
                hex(TEXT).fade(0.55),
                hex(CYAN),
            ))
            .child(self.biolume_text_clip(mode, lines, k, text_x))
            .child(at(
                self.biolume_status(mode, mode_at, k),
                (DESIGN_W - STATUS_W) / 2.0 * k,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The words, clipped under the waveform, so a line that leaves on an
    /// overflow fades out inside the bell.
    fn biolume_text_clip(&self, mode: Mode, lines: u32, k: f32, text_x: f32) -> Element<Msg> {
        // Across the whole bell: a centred line that grows slides to its
        // new middle, and a new word rides along past the text box until
        // it gets there.
        let m = 10.0 * k;
        let x0 = (BELL_X - BELL_RX + 20.0) * k;
        let y = (TEXT_TOP - CLIP_TOP) * k;
        let h = y + lines as f32 * TEXT_LINE * k + m;
        let clip = Element::layer("text-clip")
            .size(2.0 * (BELL_RX - 20.0) * k, h)
            .clip()
            .motion(Prop::Size, Motion::Snap)
            .child(at(self.biolume_text(mode, lines), text_x - x0, y));
        at(clip, x0, CLIP_TOP * k)
    }

    fn biolume_text(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let tw = l.text_width();
        let dim = if mode == Mode::Cancelled { 0.4 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(TEXT))
            .text_halo(hex(CYAN).fade(0.35), 8.0 * k)
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0 * k)));
        let paths = biolume_paths(l);
        let words = self.letter_fx_words(self.typo_words_in(words, &paths));
        // MO-81: the type breathes only while it is spoken.
        let words = if mode == Mode::Listening {
            words.wild_motion(self)
        } else {
            words
        };
        let words = self.handoff_or(words, &l.text_style(), Some(tw), hex(TEXT), hex(CYAN));
        let editing = self.edit_view.is_some();
        let text = Element::column("text")
            .width(tw)
            .height(lines as f32 * TEXT_LINE * k)
            .child(words)
            .children(self.paths_overlays(&paths).into_iter().filter(|_| !editing))
            .children(self.line_marks(
                l.text_style(),
                tw,
                endings::Ink {
                    text: hex(TEXT),
                    mark: hex(CYAN),
                    faint: hex(DIM),
                },
                endings::Cancel::Unthread,
            ));
        // MO-82: the ink spreads through the newest line's letters.
        let text = self.letter_fx_layers(text, tw);
        let text = self.signal_marks(text, &self.shown, tw, (0.0, 0.0), hex(TEXT));
        self.with_hearing(text, &l.text_style(), tw, hex(TEXT), hex(CYAN))
    }

    /// The status, centred along the bell's lower edge: a dot, the state
    /// word and the detail or hint.
    fn biolume_status(&self, mode: Mode, mode_at: f64, k: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(ui(l), STATUS_PX * k, STATUS_H * k);
        let status = self.status();
        let (state, detail, dot) = match mode {
            Mode::Done => {
                let detail = self
                    .kit_target()
                    .map(|a| format!("into {a}"))
                    .unwrap_or_default();
                (self.delivery_word().to_string(), detail, hex(GREEN))
            }
            // "Cancelled", "No speech" or "Failed": how it ended, even when
            // the daemon still reports a delivery.
            Mode::Cancelled => (
                self.ending().map_or("Cancelled", Ending::label).to_string(),
                "  ·  nothing was typed".to_string(),
                hex(DIM),
            ),
            _ => (status.state.clone(), status.detail.clone(), hex(CYAN)),
        };
        let hollow = mode == Mode::Finishing;
        let mark = Element::canvas("dot", mode_at, move |_, r, p| {
            let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            if hollow {
                p.stroke(
                    &make::ring(c, 4.6 * k),
                    &StrokeStyle::solid(1.4 * k, hex(TEXT)),
                );
            } else {
                p.shape(ShapeInstance::circle(
                    c,
                    4.6 * k,
                    Paint::glow(dot.fade(0.5), 4.0 * k, 0.5),
                ));
                p.shape(ShapeInstance::circle(c, 4.6 * k, Paint::solid(dot)));
            }
        })
        .size(12.0 * k, 12.0 * k)
        .motion(Prop::Position, Motion::Snap);
        let words = super::kit::detail_words(&state, &detail);
        let mut row = Element::row("status")
            .width(STATUS_W * k)
            .height(STATUS_H * k)
            .align_items(AlignItems::CENTER)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
            .child(
                // Keyed once, not by state: a new key per state faded the
                // old word out under the new one.
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(hex(TEXT))
                    .margin_edges(0.0, 0.0, 0.0, 9.0 * k)
                    .motion(Prop::Position, Motion::Snap),
            )
            .child(
                Element::live_text("detail", &words)
                    .text_style(style)
                    .color(hex(HINT))
                    .height(STATUS_H * k)
                    .margin_edges(0.0, 0.0, 0.0, 3.0 * k)
                    .shrink(1.0)
                    .min_width(0.0)
                    .motion(Prop::Position, Motion::Snap)
                    .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                    .word_enter(Transition::fade())
                    .word_change(Transition::fade().opacity(0.2)),
            );
        if let Some(fraction) = self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))
        {
            row = row.child(
                Element::progress(
                    "progress",
                    Progress {
                        fraction: Some(fraction),
                        track: hex(BLUE).fade(0.2),
                        bar: hex(CYAN),
                        since: self.shown_at,
                        period: 1.6,
                    },
                )
                .height(2.0 * k)
                .width(70.0 * k)
                .margin_edges(0.0, 0.0, 0.0, 12.0 * k),
            );
        }
        row
    }
}
