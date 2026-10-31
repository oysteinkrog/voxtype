//! The `orb` look of the vo popup, after concept 01, glass aurora orb: a
//! wide pill of dark violet glass with a cyan and violet rim light. At its
//! left a glowing aurora orb sits in a glass ring; it is raymarched on the
//! GPU (`vo_3d::hero`), its colours drift and it swells with the voice. To
//! the right a bar waveform runs along the top, the words sit left-aligned
//! under it and grow downward, and the status row runs along the bottom.
//!
//! While the final text is made the status icon turns into a spinner; when
//! the text lands the waveform collapses into a bright hairline that flies
//! out to the right, and the status reads the delivery and the word count.
//! A cancel greys the orb and dims the words. Under reduced motion the orb
//! rests, the bars hold still and nothing flies.

use std::f32::consts::TAU;
use std::fmt;

use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::Glass;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo_3d::sdf::{HeroInside, MAX_FOG_STEPS};
use vo_3d::{HeroFrame, HeroLook, HeroShape, HeroSource};

/// Colours sampled from the redesign.
const TINT: u32 = 0x141638;
const RIM: u32 = 0x9a8cff;
const INK: u32 = 0xf2f4ff;
const HINT: u32 = 0xb4b8e0;
const DIM: u32 = 0x6e7194;
const CYAN: u32 = 0x3ee0e8;
const BLUE: u32 = 0x4a6cff;
const VIOLET: u32 = 0x9a5cff;
const MAGENTA: u32 = 0xc65cff;
/// The orb's three drifting colours, its rim, and the grey of a cancel.
const ORB_A: u32 = 0x2b6cff;
const ORB_B: u32 = 0x8a4dff;
const ORB_C: u32 = 0x2de0c8;
const ORB_RIM: u32 = 0xd8c8ff;
const ORB_GREY: u32 = 0x5a5f78;

/// The pill in the redesign's units at `width_px = 720`; the look scales
/// with the popup's width. `ROOM` is kept round it for its glow.
const DESIGN_W: f32 = 720.0;
const PILL_W: f32 = 720.0;
const PILL_H: f32 = 240.0;
const ROOM: f32 = 20.0;
/// The orb's middle, its radius and the glass ring round it.
const ORB_X: f32 = 120.0;
const ORB_R: f32 = 64.0;
const RING_R: f32 = 80.0;
/// The orb's box: the hero's diameter, so `ORB_R` at its resting size.
const ORB_BOX: f32 = 2.0 * ORB_R / ORB_REST;
const ORB_REST: f32 = 0.9;
/// March steps per pixel: a sphere needs few.
const ORB_STEPS: u32 = 32;
/// The column right of the orb: its left edge, the waveform's middle,
/// width and half height, and its bars.
const COL_X: f32 = 236.0;
const WAVE_Y: f32 = 58.0;
const WAVE_W: f32 = 300.0;
const WAVE_HALF_H: f32 = 20.0;
const WAVE_PITCH: f32 = 6.0;
const BAR_W: f32 = 2.4;
const BAR_SECS: f64 = 0.03;
/// The words: width, top of the text box, size and pitch.
const TEXT_W: f32 = 444.0;
const TEXT_TOP: f32 = 88.0;
const TEXT_PX: f32 = 30.0;
const TEXT_LINE: f32 = 38.0;
pub(crate) const MAX_LINES: u32 = 2;
/// The status row's middle and height, and the progress line above it.
const STATUS_Y: f32 = 200.0;
const STATUS_H: f32 = 20.0;
const PROGRESS_W: f32 = 200.0;
const PROGRESS_GAP: f32 = 16.0;
/// Idle (RD-02): the aurora's turns a second and the rim highlight's
/// period.
const SWIRL_TURNS_S: f32 = 0.1;
const RIM_PERIOD_S: f32 = 7.0;
/// The frame pace while the popup records room tone (RD-11): the swirl
/// turns 4.5 degrees a frame at this pace.
pub(crate) const ROOM_TONE_FPS: f64 = 8.0;
/// How long the hairline flies once the text lands.
const SEND_SECS: f64 = 0.7;

/// R2-01-a Sonic ripple: a thin ring per new word grows from the orb's rim
/// across the glass behind the words and fades within `RIPPLE_SECS`; its
/// alpha follows the word's peak level. Under reduced motion the orb's
/// ring brightens once per word instead.
pub(crate) const SONIC_RIPPLE: &str = "sonic-ripple";
const RIPPLE_SECS: f64 = 1.2;
const MAX_RIPPLES: usize = 3;
const RIPPLE_ALPHA: f32 = 0.15;
const RIPPLE_REACH: f32 = 560.0;
/// R2-01-b Magnetic type: tiny copies of the new word's letters stream
/// from the orb along a lane just under the newest line and fade as they
/// reach their letter, all within `STREAM_SECS`. The word shows at
/// `STREAM_INK` from its first frame. Under reduced motion no stream.
pub(crate) const MAGNETIC_TYPE: &str = "magnetic-type";
const STREAM_SECS: f64 = 0.4;
const STREAM_STAGGER: f64 = 0.12;
const STREAM_MIN: usize = 20;
const STREAM_MAX: usize = 40;
const STREAM_ALPHA: f32 = 0.3;
const STREAM_SCALE: f32 = 0.3;
const STREAM_LANE: f32 = 9.0;
const STREAM_INK: f32 = 0.7;
/// A word that opens a line waits for the line to scroll or grow in.
const LINE_WAIT: f64 = 0.45;
/// R2-01-c Breathing light: the aurora inside the orb grows from 0.95 to
/// 1.05 and brightens with the voice, smoothed with `BREATH_ATTACK` and
/// `BREATH_RELEASE`; a faint light spills behind the newest line. Under
/// reduced motion the brightness only.
pub(crate) const BREATHING_LIGHT: &str = "breathing-light";
const BREATH_ATTACK: f64 = 0.08;
const BREATH_RELEASE: f64 = 0.6;
const BREATH_TICK: f64 = 1.0 / 60.0;
const BREATH_LOOK_BACK: f64 = 2.0;
const SPILL_ALPHA: f32 = 0.08;
const SPILL_W: f32 = 240.0;

const _: () = assert!(RIPPLE_ALPHA <= 0.15 && STREAM_ALPHA <= 0.3 && SPILL_ALPHA < 0.1);
const _: () = assert!(STREAM_STAGGER < STREAM_SECS);

/// R2-01-c: the voice, 0 to 1, from `level` (the raw RMS).
fn voice(level: f32) -> f32 {
    (level * 1.6).min(1.0)
}

/// R2-01-c: the voice at `t` smoothed with a fast attack and a slow
/// release, stepped at a fixed tick over the last `BREATH_LOOK_BACK`
/// seconds: a pure function of the levels and `t`.
fn breath(levels: &Levels, t: f64) -> f32 {
    let up = 1.0 - (-BREATH_TICK / BREATH_ATTACK).exp() as f32;
    let down = 1.0 - (-BREATH_TICK / BREATH_RELEASE).exp() as f32;
    let n = (BREATH_LOOK_BACK / BREATH_TICK) as i32;
    let mut env = 0.0f32;
    for i in (0..=n).rev() {
        let v = voice(levels.at(t - i as f64 * BREATH_TICK));
        let rate = if v > env { up } else { down };
        env += (v - env) * rate;
    }
    env
}

/// R2-01-c: the aurora's size for a smoothed voice `env`.
fn aurora_scale(env: f32) -> f32 {
    0.95 + 0.1 * env.clamp(0.0, 1.0)
}

/// R2-01-a: the rings alive at `t` for words born at `times` (oldest
/// first), each with how far it has run (0 to 1). At most `MAX_RIPPLES`,
/// the newest.
fn ripples(t: f64, times: &[f64]) -> Vec<(usize, f32)> {
    let alive: Vec<(usize, f32)> = times
        .iter()
        .enumerate()
        .filter(|(_, &at)| t >= at && t - at < RIPPLE_SECS)
        .map(|(i, &at)| (i, ((t - at) / RIPPLE_SECS) as f32))
        .collect();
    let skip = alive.len().saturating_sub(MAX_RIPPLES);
    alive[skip..].to_vec()
}

/// R2-01-b: copy `j` of `n` at `age` seconds after the word: how far along
/// its path (0 to 1) and its alpha; `None` before it leaves or once it
/// has arrived.
fn stream(age: f64, j: usize, n: usize) -> Option<(f32, f32)> {
    let start = STREAM_STAGGER * j as f64 / (n.max(2) - 1) as f64;
    let u = ((age - start) / (STREAM_SECS - STREAM_STAGGER)) as f32;
    if !(0.0..1.0).contains(&u) {
        return None;
    }
    Some((u, STREAM_ALPHA * (5.0 * u).min(1.0) * (1.0 - u)))
}

// The orb and its ring sit in the pill left of the column; the words sit
// between the waveform and the status row, inside the pill.
const _: () = assert!(ORB_X + RING_R + 20.0 < COL_X);
const _: () = assert!(PILL_H / 2.0 - RING_R > 20.0);
const _: () = assert!(TEXT_TOP > WAVE_Y + WAVE_HALF_H + 4.0);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < STATUS_Y - PROGRESS_GAP);
const _: () = assert!(STATUS_Y + STATUS_H / 2.0 < PILL_H - 20.0);
const _: () = assert!(COL_X + TEXT_W < PILL_W - 36.0);

/// MO-84 in orb: a cool light with the look's cyan as its accent, and
/// soft shadows and caustics on the dark violet glass. The light adds its
/// colour (alpha is not read), so it is dimmed toward black, and its glow
/// is a small spot: the text box is a line or two tall, and a wide glow
/// lights the whole box as a grey strip past the words.
pub(crate) fn letter_light() -> vo_3d::glyph_fx::Light {
    vo_3d::glyph_fx::Light {
        color: mix(Color::BLACK, hex(INK), 0.25),
        accent: hex(CYAN),
        shadow_len: 14.0,
        shadow: 0.15,
        caustics: 0.3,
        falloff: 100.0,
        sheen: 0.35,
        edge_fade: 40.0,
        ..vo_3d::glyph_fx::Light::default()
    }
}

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn sans(look: &Look) -> u32 {
    look.fonts.system("Google Sans Flex").unwrap_or(SANS)
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = scale(look);
    TextStyle::new(sans(look), TEXT_PX * k, TEXT_LINE * k).tracking(-0.1 * k)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    Size::new(
        ((PILL_W + 2.0 * ROOM) * k).ceil(),
        ((PILL_H + 2.0 * ROOM) * k).ceil(),
    )
}

/// The pill's top in the surface: below the room for its glow.
pub(crate) fn panel_top(look: &Look) -> f32 {
    ROOM * scale(look)
}

/// The middle of the newest line below the surface's top. The words grow
/// downward from the top of the text box.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (ROOM + TEXT_TOP + (lines.clamp(1, MAX_LINES) as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// The aurora orb: light from inside, three colours drifting over it, a
/// violet halo. It swells and ripples with the voice and settles once the
/// words end.
#[derive(Clone)]
struct Orb {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    still: bool,
    /// R2-01-c: the aurora breathes with the smoothed voice.
    breathing: bool,
}

impl fmt::Debug for Orb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Orb")
            .field("mode", &self.mode)
            .field("still", &self.still)
            .finish()
    }
}

impl Orb {
    /// How long the orb takes to settle once the words end.
    const SETTLE_SECS: f64 = 0.8;

    fn moving(&self) -> bool {
        !self.still && matches!(self.mode, Mode::Loading | Mode::Listening | Mode::Finishing)
    }
}

impl HeroSource for Orb {
    fn frame(&self, t: f64, _size: Size) -> HeroFrame {
        // Reduced motion: one resting orb. Once the words end the ripples
        // ease out and the colours hold where they settled.
        let (phase, calm) = if self.still {
            (0.0, 0.0)
        } else if self.moving() {
            ((t - self.shown_at) as f32, 1.0)
        } else {
            let end = t.min(self.mode_at + Self::SETTLE_SECS);
            let calm = 1.0 - ease_out((t - self.mode_at) / Self::SETTLE_SECS);
            ((end - self.shown_at) as f32, calm)
        };
        let voice = if self.mode == Mode::Listening && !self.still {
            voice(self.levels.at(t))
        } else {
            0.0
        };
        // R2-01-c: the smoothed voice sizes and lights the aurora; under
        // reduced motion it lights it only.
        let env = if self.breathing && self.mode == Mode::Listening {
            breath(&self.levels, t)
        } else {
            0.0
        };
        let (fog_scale, fog_lift) = if !self.breathing {
            (1.2, 1.0 * voice)
        } else if self.still {
            (1.2, 1.6 * env)
        } else {
            (1.2 / aurora_scale(env), 1.6 * env)
        };
        let grey = self.mode == Mode::Cancelled;
        let c = |c: u32| if grey { hex(ORB_GREY) } else { hex(c) };
        HeroFrame {
            shape: HeroShape::Orb,
            look: HeroLook::Glow {
                a: c(ORB_A),
                b: c(ORB_B),
                c: c(ORB_C),
                rim: c(ORB_RIM),
                body: 0.85,
                halo: if grey { 0.15 } else { 0.3 + 0.35 * voice },
                halo_width: 0.1,
            },
            stretch: [ORB_REST + 0.05 * voice; 3],
            wobble: (0.05 + 0.3 * voice) * calm,
            phase: 0.6 * phase,
            // Idle (RD-02): the aurora swirls a tenth of a turn a second.
            spin: TAU * SWIRL_TURNS_S * phase,
            steps: ORB_STEPS,
            // Curtains of aurora folding inside a thin glass skin.
            inside: Some(HeroInside {
                skin: 0.5,
                core: Color::WHITE,
                core_glow: 0.0,
                fog_a: c(ORB_C),
                fog_b: c(ORB_B),
                fog: if grey { 0.6 } else { 1.5 + fog_lift },
                fog_scale,
                fog_steps: MAX_FOG_STEPS,
            }),
        }
    }

    fn until(&self) -> f64 {
        // Reduced motion with the breathing light: a still orb whose aurora
        // still brightens with the voice.
        if self.moving() || (self.breathing && self.still && self.mode == Mode::Listening) {
            f64::INFINITY
        } else {
            self.mode_at + Self::SETTLE_SECS + 0.05
        }
    }
}

/// The glass ring round the orb: a thin lit edge and a soft inner shade.
#[derive(Clone, Copy)]
struct Ring {
    k: f32,
    lit: f32,
}

impl Ring {
    fn draw(&self, r: Rect, p: &mut Painter) {
        let k = self.k;
        let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rad = RING_R * k;
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::glow(hex(VIOLET).fade(0.25 * self.lit), 14.0 * k, 0.6),
        ));
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::linear(
                Point::new(c.x, c.y - rad),
                Color::hex(0x2a2f66, 0.55),
                Point::new(c.x, c.y + rad),
                Color::hex(0x0c0e26, 0.75),
            ),
        ));
        let edge = StrokeStyle::solid(1.4 * k, hex(RIM))
            .linear(
                Point::new(c.x - rad, c.y - rad),
                hex(CYAN).fade(0.2 + 0.6 * self.lit),
                Point::new(c.x + rad, c.y + rad),
                hex(VIOLET).fade(0.2 + 0.6 * self.lit),
            )
            .glow(3.0 * k);
        p.stroke(&make::ring(c, rad), &edge);
    }
}

/// The bar waveform: the newest level at the right, cyan at the left
/// through blue to violet at the right, thinning to dots at both ends.
/// When the text lands it collapses into a hairline that flies out right.
#[derive(Clone)]
struct Wave {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    k: f32,
    still: bool,
}

impl Wave {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing if !self.still => f64::INFINITY,
            Mode::Done if !self.still => self.mode_at + SEND_SECS + 0.1,
            _ => self.mode_at + 0.6,
        }
    }

    fn colour(x: f32) -> Color {
        if x < 0.5 {
            mix(hex(CYAN), hex(BLUE), x * 2.0)
        } else {
            mix(hex(BLUE), hex(MAGENTA), (x - 0.5) * 2.0)
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let (x0, cy) = (r.x, r.y + r.h / 2.0);
        let n = (WAVE_W / WAVE_PITCH).floor() as i32;
        if self.mode == Mode::Done {
            self.send(t, x0, cy, p);
            return;
        }
        let settle = match self.mode {
            Mode::Finishing | Mode::Cancelled if !self.still => ease_out((t - self.mode_at) / 0.5),
            Mode::Finishing | Mode::Cancelled => 1.0,
            _ => 0.0,
        };
        let dim = if self.mode == Mode::Cancelled {
            0.35
        } else {
            1.0 - 0.3 * settle
        };
        let t_eff = if self.still {
            self.shown_at
        } else {
            self.finish_at.map_or(t, |f| t.min(f))
        };
        let sweep = ((t - self.shown_at) / 1.4).fract() as f32;
        for i in 0..=n {
            let across = i as f32 / n as f32;
            let x = x0 + i as f32 * WAVE_PITCH * k;
            let c = Self::colour(across).fade(dim);
            if self.mode == Mode::Loading {
                // A soft light runs along the dots while the model loads.
                let near = if self.still {
                    0.0
                } else {
                    (1.0 - (across - sweep).abs() * 6.0).clamp(0.0, 1.0)
                };
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    (1.1 + near) * k,
                    Paint::solid(c.fade(0.4 + 0.6 * near)),
                ));
                continue;
            }
            let level = if self.still {
                // Reduced motion: a fixed low shape, not the voice.
                0.25
            } else {
                self.levels.at(t_eff - (n - i) as f64 * BAR_SECS)
            };
            // Thin at both ends, fullest a little right of the middle.
            let taper = (1.0 - ((across - 0.55) / 0.55).abs().powf(2.2)).max(0.0);
            let half = (level * 1.9).min(1.0) * taper * WAVE_HALF_H * k * (1.0 - settle);
            if half < 1.8 * k {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.1 * k,
                    Paint::solid(c.fade(0.7)),
                ));
                continue;
            }
            let bar = Rect::new(x - BAR_W * k / 2.0, cy - half, BAR_W * k, 2.0 * half);
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W * k / 2.0),
                Paint::glow(c.fade(0.45), 5.0 * k, 0.6),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W * k / 2.0),
                Paint::linear(
                    Point::new(x, cy - half),
                    mix(c, hex(INK), 0.3),
                    Point::new(x, cy),
                    c,
                ),
            ));
        }
    }

    /// The hand-off: a bright hairline from the waveform's left end, its
    /// head flying out past the right end with an arrow, fading as it goes.
    fn send(&self, t: f64, x0: f32, cy: f32, p: &mut Painter) {
        let k = self.k;
        let s = if self.still {
            0.6
        } else {
            ((t - self.mode_at) / SEND_SECS).clamp(0.0, 1.0) as f32
        };
        let go = ease_out(s as f64);
        let fade = if self.still { 1.0 } else { 1.0 - s.powi(3) };
        if fade <= 0.0 {
            return;
        }
        let head = x0 + (WAVE_W * 0.5 + (WAVE_W * 0.5 + 60.0) * go) * k;
        let tail = x0 + WAVE_W * 0.6 * go * k;
        p.shape(ShapeInstance::rrect(
            RRect::new(
                Rect::new(tail, cy - 0.8 * k, (head - tail).max(0.0), 1.6 * k),
                0.8 * k,
            ),
            Paint::linear(
                Point::new(tail, cy),
                hex(VIOLET).fade(0.0),
                Point::new(head, cy),
                hex(CYAN).fade(fade),
            ),
        ));
        p.shape(ShapeInstance::circle(
            Point::new(head, cy),
            3.0 * k,
            Paint::glow(hex(CYAN).fade(0.9 * fade), 8.0 * k, 0.6),
        ));
        let a = 8.0 * k;
        let arrow = Contour::polyline(
            &[
                Point::new(head + 4.0 * k + a, cy),
                Point::new(head + 4.0 * k, cy - a * 0.7),
                Point::new(head + 4.0 * k + a, cy),
                Point::new(head + 4.0 * k, cy + a * 0.7),
            ],
            false,
        );
        p.stroke(
            &arrow,
            &StrokeStyle::solid(1.4 * k, hex(INK).fade(0.8 * fade)),
        );
    }
}

impl Popup {
    pub(crate) fn orb_view(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let s = surface(l);
        let root = Element::column("root").size(s.w, s.h);
        if self.closing {
            return root;
        }
        root.child(at(self.orb_pill(), ROOM * k, ROOM * k))
    }

    /// The glass pill with everything on it.
    fn orb_pill(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let still = self.still();
        let (w, h) = (PILL_W * k, PILL_H * k);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let glass = Glass {
            accent: hex(VIOLET),
            rim: 1.2,
            bevel: 14.0 * k,
            shadow: [0.45, 22.0 * k, 10.0 * k],
            ..Glass::dark()
        };
        let glass = glass
            .tinted_for(hex(TINT), &[hex(INK), hex(HINT)], 4.5)
            .unwrap_or(Glass {
                tint: Color::hex(TINT, 0.9),
                ..glass
            });
        let glass = if still { glass.still() } else { glass };
        let wave = Wave {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            finish_at: self.extra.finish_at,
            k,
            still,
        };
        let wave_box = Size::new((WAVE_W + 80.0) * k, 2.0 * (WAVE_HALF_H + 10.0) * k);
        let until = wave.until();
        let ring = Ring {
            k,
            lit: if mode == Mode::Cancelled { 0.2 } else { 1.0 },
        };
        let ring_box = 2.0 * (RING_R + 18.0) * k;
        let orb = Orb {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            still,
            breathing: l.on(BREATHING_LIGHT),
        };
        let d = ORB_BOX * k;
        Element::panel("pill")
            .size(w, h)
            .radius(h / 2.0)
            .glass(glass)
            .border(Color::hex(RIM, 0.55), 1.2 * k)
            .glow(hex(VIOLET).fade(0.35), 16.0 * k, 0.6)
            // Idle (RD-02): a highlight runs round the rim every 7 s.
            .rim(
                hex(CYAN).fade(0.7),
                1.6 * k,
                0.25,
                -0.3,
                if still { 0.0 } else { RIM_PERIOD_S },
            )
            .presence(Transition::fade().offset(0.0, 8.0).scale(0.98), exit)
            .child(at(
                Element::canvas("ring", mode_at + 0.1, move |_, r, p| ring.draw(r, p))
                    .size(ring_box, ring_box),
                ORB_X * k - ring_box / 2.0,
                h / 2.0 - ring_box / 2.0,
            ))
            .child(at(
                Element::gpu("orb", vo_3d::hero(orb)).size(d, d),
                ORB_X * k - d / 2.0,
                h / 2.0 - d / 2.0,
            ))
            .children(self.orb_ripples(mode, Size::new(w, h), k))
            .children(self.orb_spill(mode, Size::new(w, h), k))
            .children(self.orb_stream(mode, Size::new(w, h), k))
            .child(at(
                Element::canvas("wave", until, move |t, r, p| {
                    wave.draw(t, Rect::new(r.x, r.y, WAVE_W * k, r.h), p)
                })
                .size(wave_box.w, wave_box.h),
                COL_X * k,
                WAVE_Y * k - wave_box.h / 2.0,
            ))
            .children(self.limit_line(
                Point::new(COL_X * k, TEXT_TOP * k),
                Size::new(w, h),
                hex(INK).fade(0.55),
                hex(CYAN),
            ))
            .child(self.orb_text(mode, k))
            .children(self.orb_progress(mode, k))
            .child(at(
                self.orb_status(mode, mode_at, k),
                COL_X * k,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The words, left-aligned, growing downward from the top of the box.
    fn orb_text(&self, mode: Mode, k: f32) -> Element<Msg> {
        let l = &self.look;
        let tw = text_width(l);
        let lines = self.lines.clamp(1, MAX_LINES);
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(INK))
            .opacity(dim)
            .width(tw)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(if l.on(MAGNETIC_TYPE) {
                // R2-01-b: the word is legible from its first frame.
                Transition::fade().opacity(STREAM_INK)
            } else {
                Transition::fade().opacity(0.0).offset(0.0, 6.0)
            }));
        // MO-84 light through the letters, then MO-81 breath last.
        let words = self
            .letter_fx_words(self.typo_words(words))
            .wild_motion(self);
        let words = self.handoff_or(words, &l.text_style(), Some(tw), hex(INK), hex(CYAN));
        let text = Element::column("text")
            .width(tw)
            .height(lines as f32 * TEXT_LINE * k)
            .child(words)
            .children(self.line_marks(
                l.text_style(),
                tw,
                endings::Ink {
                    text: hex(INK),
                    mark: hex(CYAN),
                    faint: hex(DIM),
                },
                endings::Cancel::Unthread,
            ));
        let text = self.typo_marks(text, tw);
        let text = self.letter_fx_layers(text, tw);
        let text = self.signal_marks(text, &self.shown, tw, (0.0, 0.0), hex(INK));
        at(
            self.with_hearing(text, &l.text_style(), tw, hex(INK), hex(CYAN)),
            COL_X * k,
            TEXT_TOP * k,
        )
    }

    /// The newest word's box in the pill, its baseline, and when its
    /// effects start.
    fn orb_newest(&self, k: f32) -> Option<(Rect, f32, f64)> {
        let l = &self.look;
        let style = l.text_style();
        let rows = self.wrap_lines(&self.shown, &style, text_width(l));
        let row = rows.len().min(MAX_LINES as usize).checked_sub(1)?;
        let last = rows.last()?;
        let newest = last.last()?;
        let mut born = self.heard.word(newest.id)?.born;
        if rows.len() > 1 && last.len() == 1 {
            born += LINE_WAIT;
        }
        let (boxes, _) = sounding::word_boxes(&self.measure, last, &style, None);
        let b = boxes.iter().find(|b| b.id == newest.id)?;
        let top = (TEXT_TOP + row as f32 * TEXT_LINE) * k;
        let rect = Rect::new(COL_X * k + b.rect.x, top + b.rect.y, b.rect.w, b.rect.h);
        Some((rect, top + b.baseline, born))
    }

    /// R2-01-a: a ring per new word from the orb's rim across the glass,
    /// behind the words and clipped to the pill. Under reduced motion the
    /// orb's ring brightens once per word.
    fn orb_ripples(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        if !self.look.on(SONIC_RIPPLE) || mode != Mode::Listening || self.edit_view.is_some() {
            return None;
        }
        let levels = self.kit_raw_levels();
        let words: Vec<(f64, f32)> = self
            .shown
            .iter()
            .rev()
            .take(6)
            .rev()
            .filter_map(|w| {
                let h = self.heard.word(w.id)?;
                let (s0, s1) = if h.sound.1 > h.sound.0 {
                    h.sound
                } else {
                    (h.born - 0.3, h.born)
                };
                let n = ((s1 - s0) / 0.02).ceil().max(1.0) as usize;
                let peak = (0..=n)
                    .map(|i| voice(levels.at(s0 + (s1 - s0) * i as f64 / n as f64)))
                    .fold(0.0f32, f32::max);
                Some((h.born, 0.35 + 0.65 * peak))
            })
            .collect();
        let newest = words.iter().map(|w| w.0).fold(f64::NEG_INFINITY, f64::max);
        if self.extra.now - newest >= RIPPLE_SECS {
            return None;
        }
        let still = self.still();
        let times: Vec<f64> = words.iter().map(|w| w.0).collect();
        let centre = Point::new(ORB_X * k, size.h / 2.0);
        let canvas = Element::canvas("ripples", newest + RIPPLE_SECS, move |t, r, p| {
            let c = Point::new(r.x + centre.x, r.y + centre.y);
            for (i, u) in ripples(t, &times) {
                let a = RIPPLE_ALPHA * words[i].1 * (1.0 - u).powf(1.5);
                if still {
                    let edge = StrokeStyle::solid(1.4 * k, hex(CYAN).fade(a)).glow(4.0 * k);
                    p.stroke(&make::ring(c, RING_R * k), &edge);
                    continue;
                }
                let rad = (ORB_R + (RIPPLE_REACH - ORB_R) * ease_out(u as f64)) * k;
                let edge = StrokeStyle::solid(1.0 * k, hex(CYAN).fade(a))
                    .linear(
                        Point::new(c.x, c.y - rad),
                        hex(CYAN).fade(a),
                        Point::new(c.x + rad, c.y),
                        hex(VIOLET).fade(a),
                    )
                    .glow(2.0 * k);
                p.stroke(&make::ring(c, rad), &edge);
            }
        });
        let layer = Element::layer("ripples-clip")
            .size(size.w, size.h)
            .radius(size.h / 2.0)
            .clip()
            .child(at(canvas.size(size.w, size.h), 0.0, 0.0));
        Some(at(layer, 0.0, 0.0))
    }

    /// R2-01-c: the aurora's light spilling faintly behind the newest line,
    /// as bright as the smoothed voice.
    fn orb_spill(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        if !self.look.on(BREATHING_LIGHT) || mode != Mode::Listening || self.edit_view.is_some() {
            return None;
        }
        let levels = self.kit_raw_levels();
        let lines = self.lines.clamp(1, MAX_LINES) as f32;
        let mid = (TEXT_TOP + (lines - 0.5) * TEXT_LINE) * k;
        // The light has died down a few release times after the last level.
        let until = self.newest_at + 6.0 * BREATH_RELEASE;
        let canvas = Element::canvas("spill", until, move |t, r, p| {
            let a = SPILL_ALPHA * breath(&levels, t);
            if a < 0.003 {
                return;
            }
            // Soft pools of light fading out along the line from the orb's
            // side: no edge to read as a mark.
            let rad = TEXT_LINE * k;
            for i in 0..4 {
                let x = r.x + (COL_X - 30.0 + i as f32 * SPILL_W / 4.0) * k;
                let fade = 1.0 - i as f32 / 4.0;
                p.shape(ShapeInstance::circle(
                    Point::new(x, r.y + mid),
                    rad,
                    Paint::glow(hex(ORB_C).fade(a * fade), rad, 0.5),
                ));
            }
        });
        Some(at(canvas.size(size.w, size.h), 0.0, 0.0))
    }

    /// R2-01-b: tiny copies of the new word's letters stream from the orb
    /// along a lane under the newest line and fade as they reach their
    /// letter. Behind the words; none under reduced motion.
    fn orb_stream(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(MAGNETIC_TYPE)
            || mode != Mode::Listening
            || self.still()
            || self.edit_view.is_some()
        {
            return None;
        }
        let (rect, baseline, born) = self.orb_newest(k)?;
        if self.extra.now - born >= STREAM_SECS {
            return None;
        }
        let word = self.shown.last()?;
        let style = l.text_style();
        let shaped = self.measure.layout(&word.text, &style, None);
        let outline = vo::scene::glyph_morph::WordOutline::new(
            self.measure.fonts(),
            &shaped,
            style.size,
            Point::ZERO,
        );
        let letters: Arc<[(f32, Arc<[Contour]>)]> = outline
            .glyphs
            .iter()
            .filter(|g| g.ink.h > 0.0)
            .map(|g| {
                let (cx, cy) = (g.ink.x + g.ink.w / 2.0, g.ink.y + g.ink.h / 2.0);
                let m = Affine::translate(-cx, -cy).then(&Affine::scale(STREAM_SCALE));
                let ink: Arc<[Contour]> =
                    g.outline.contours.iter().map(|c| c.transform(&m)).collect();
                (cx, ink)
            })
            .collect();
        if letters.is_empty() {
            return None;
        }
        let n = (letters.len() * 5).clamp(STREAM_MIN, STREAM_MAX);
        let lane = baseline + STREAM_LANE * k;
        let orb = Point::new(ORB_X * k, size.h / 2.0);
        let canvas = Element::canvas("stream", born + STREAM_SECS, move |t, r, p| {
            let age = t - born;
            for j in 0..n {
                let Some((u, a)) = stream(age, j, n) else {
                    continue;
                };
                let (x, ink) = &letters[j % letters.len()];
                // Leave the orb's rim fanned a little, sweep into the lane
                // under the line, then rise to just under the letter.
                let fan = 0.5 * (j as f32 / (n - 1) as f32 - 0.5);
                let from = Point::new(orb.x + ORB_R * k * fan.cos(), orb.y + ORB_R * k * fan.sin());
                let to = Point::new(rect.x + x, baseline + 5.0 * k);
                let bend = Point::new(COL_X * k, lane);
                let v = ease_out(u as f64);
                let q = |a: f32, b: f32, c: f32| {
                    (1.0 - v) * (1.0 - v) * a + 2.0 * (1.0 - v) * v * b + v * v * c
                };
                let pos = Point::new(r.x + q(from.x, bend.x, to.x), r.y + q(from.y, lane, to.y));
                let m = Affine::translate(pos.x, pos.y);
                let moved: Vec<Contour> = ink.iter().map(|c| c.transform(&m)).collect();
                p.fill(&moved, &StrokeStyle::solid(1.0, hex(INK).fade(a)));
            }
        });
        Some(at(canvas.size(size.w, size.h), 0.0, 0.0))
    }

    /// A thin line over the status while a model loads or the final text
    /// is made.
    fn orb_progress(&self, mode: Mode, k: f32) -> Option<Element<Msg>> {
        let fraction = self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))?;
        Some(at(
            Element::progress(
                "progress",
                Progress {
                    fraction: Some(fraction),
                    track: Color::hex(RIM, 0.2),
                    bar: hex(CYAN),
                    since: self.shown_at,
                    period: 1.6,
                },
            )
            .size(PROGRESS_W * k, 2.0 * k)
            .presence(Transition::fade(), Transition::fade()),
            COL_X * k,
            (STATUS_Y - PROGRESS_GAP) * k,
        ))
    }

    /// The status row, left-aligned under the words: an icon (a cyan
    /// microphone, a spinner while the final text is made, a check in a
    /// ring, a cross), the state word and the detail.
    fn orb_status(&self, mode: Mode, mode_at: f64, k: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(sans(l), 16.0 * k, STATUS_H * k);
        let hint = TextStyle::new(sans(l), 15.0 * k, STATUS_H * k);
        let status = self.status();
        let (state, detail) = match mode {
            Mode::Done => {
                let n = self.shown.len();
                let count = format!("  ·  {n} word{}", if n == 1 { "" } else { "s" });
                (self.delivery_word().to_string(), count)
            }
            // "Cancelled", "No speech" or "Failed": how it ended, even when
            // the daemon still reports a delivery.
            Mode::Cancelled => (
                self.ending().map_or("Cancelled", Ending::label).to_string(),
                "  ·  nothing was typed".to_string(),
            ),
            _ => (status.state.clone(), status.detail.clone()),
        };
        let still = self.still();
        let until = match mode {
            Mode::Finishing | Mode::Loading if !still => f64::INFINITY,
            _ => mode_at + 0.4,
        };
        let icon = Element::canvas("icon", until, move |t, r, p| {
            let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            let grow = if still {
                1.0
            } else {
                ease_out((t - mode_at) / 0.3)
            };
            let glyph = |outline: Outline, size: f32, colour: Color, trim: f32| {
                let unit = size / 24.0;
                let o = outline.transform(
                    &Affine::translate(-12.0, -12.0)
                        .then(&Affine::scale(unit))
                        .then(&Affine::translate(c.x, c.y)),
                );
                let len = o.length();
                (
                    o.trim(0.0, len * trim),
                    StrokeStyle::solid(2.2 * unit, colour),
                )
            };
            let (o, style) = match mode {
                Mode::Finishing | Mode::Loading => {
                    let rad = 7.0 * k;
                    p.stroke(
                        &make::ring(c, rad),
                        &StrokeStyle::solid(1.8 * k, hex(BLUE).fade(0.25)),
                    );
                    let spin = if still { 0.0 } else { t - mode_at };
                    p.stroke(
                        &make::spinner(c, rad, spin),
                        &StrokeStyle::solid(1.8 * k, hex(CYAN)).glow(3.0 * k),
                    );
                    return;
                }
                Mode::Done => {
                    p.shape(ShapeInstance::circle(
                        c,
                        8.5 * k,
                        Paint::solid(hex(HINT).fade(0.9)),
                    ));
                    glyph(
                        Outline::new(&icons::check(), 0.05),
                        12.0 * k,
                        hex(TINT),
                        grow,
                    )
                }
                Mode::Cancelled => glyph(Outline::new(&icons::x(), 0.05), 14.0 * k, hex(DIM), grow),
                Mode::Listening => {
                    glyph(Outline::new(&icons::mic(), 0.05), 20.0 * k, hex(CYAN), 1.0)
                }
            };
            p.outline(&o, &style);
        })
        .size(22.0 * k, 22.0 * k);
        let words = super::kit::detail_words(&state, &detail);
        let (state_c, detail_c) = match mode {
            Mode::Done => (hex(HINT), hex(HINT)),
            Mode::Cancelled => (hex(HINT), hex(DIM)),
            _ => (hex(HINT), mix(hex(DIM), hex(HINT), 0.6)),
        };
        Element::row("status")
            .width(TEXT_W * k)
            .height(STATUS_H * k)
            .align_items(AlignItems::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(icon)
            .child(
                // Keyed once, not by state: a new key per state faded the
                // old word out under the new one.
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(state_c)
                    .margin_edges(0.0, 0.0, 0.0, 8.0 * k),
            )
            .child(
                Element::live_text("detail", &words)
                    .text_style(hint)
                    .color(detail_c)
                    .height(STATUS_H * k)
                    .margin_edges(0.0, 0.0, 0.0, 3.0 * k)
                    .shrink(1.0)
                    .min_width(0.0)
                    .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                    .word_enter(Transition::fade())
                    .word_change(Transition::fade().opacity(0.2)),
            )
    }
}

#[cfg(test)]
mod r2_tests {
    use super::*;

    fn levels(v: f32, secs: f64) -> Levels {
        let n = (secs * FRAME_HZ as f64) as usize;
        Levels {
            levels: vec![v; n].into(),
            newest_at: secs,
        }
    }

    #[test]
    fn one_ring_per_word_and_none_after_its_time() {
        let times = [1.0, 1.1, 1.2, 1.3];
        let alive = ripples(1.35, &times);
        assert_eq!(alive.len(), MAX_RIPPLES);
        assert_eq!(alive.last().map(|r| r.0), Some(3));
        assert_eq!(ripples(1.05, &times).len(), 1);
        assert!(ripples(0.9, &times).is_empty());
        assert!(ripples(1.3 + RIPPLE_SECS, &times).is_empty());
    }

    #[test]
    fn the_letters_are_gone_after_their_time() {
        for n in [STREAM_MIN, STREAM_MAX] {
            for j in 0..n {
                assert!(stream(STREAM_SECS, j, n).is_none());
                assert!(stream(-0.01, j, n).is_none());
                if let Some((_, a)) = stream(0.2, j, n) {
                    assert!(a <= STREAM_ALPHA);
                }
            }
            // Every copy leaves, and the last one is near zero as it lands.
            let (_, a) = stream(STREAM_SECS - 1e-3, n - 1, n).unwrap();
            assert!(a < 0.01);
        }
    }

    #[test]
    fn the_aurora_follows_the_smoothed_voice_only() {
        let quiet = levels(0.0, 3.0);
        let loud = levels(1.0, 3.0);
        assert_eq!(breath(&quiet, 3.0), 0.0);
        assert_eq!(breath(&loud, 3.0), breath(&loud, 3.0));
        assert!(breath(&loud, 3.0) > 0.99);
        assert_eq!(aurora_scale(0.0), 0.95);
        assert!((aurora_scale(breath(&loud, 3.0)) - 1.05).abs() < 1e-3);
        // Fast up, slow down: 0.2 s into a sound it is most of the way up,
        // and 0.2 s after one, most of it is still there.
        let hz = FRAME_HZ as usize;
        let start = Levels {
            levels: [vec![0.0; 27 * hz / 10], vec![1.0; 3 * hz / 10]]
                .concat()
                .into(),
            newest_at: 3.0,
        };
        assert!(breath(&start, 2.9) > 0.8);
        let stop = Levels {
            levels: [vec![1.0; 27 * hz / 10], vec![0.0; 3 * hz / 10]]
                .concat()
                .into(),
            newest_at: 3.0,
        };
        assert!(breath(&stop, 2.9) > 0.6);
    }
}
