//! The `hologram` look of the vo popup, after concept 20, floating hologram
//! text: no panel, only light. A glowing ring pedestal sits at the bottom
//! with the status on it, a faint column of projected light rises from it
//! with fine scan lines, the waveform stands at its base, the words float
//! in the column in translucent blue type with a soft glow, and a glass
//! microphone orb hangs at the top.
//!
//! The words start under the orb; the wave and the ring sit below the last
//! line, so the projection grows down as lines come.
//! While the final text is made the wave lowers and the ring says so; a
//! cancel greys the light and dims the words. Under reduced motion the scan
//! lines and the ring's shimmer stand still.

use super::glyph_paths::PathsText;
use super::kit::{at, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::depth_stack::{DepthStack, Place, Sheet};
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect, Rect};
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};

/// Colours sampled from the redesign.
const ICE: u32 = 0xdcecff;
const TEXT: u32 = 0xc6dcff;
const HALO: u32 = 0x3d7bff;
const CYAN: u32 = 0x3ad2ff;
const BLUE: u32 = 0x3d6bff;
const VIOLET: u32 = 0xa064ff;
const ORB: u32 = 0x0b1636;
const HINT: u32 = 0xa9c2f0;
const DIM: u32 = 0x67708a;
const GREEN: u32 = 0x39d98a;

/// The projection in the redesign's units at `width_px = 600`.
const DESIGN_W: f32 = 600.0;
/// The microphone orb.
const ORB_Y: f32 = 82.0;
const ORB_R: f32 = 42.0;
/// The column of light, from under the orb down to the ring.
const BEAM_X0: f32 = 78.0;
const BEAM_X1: f32 = 522.0;
const BEAM_TOP: f32 = 140.0;
const CAP_RY: f32 = 16.0;
/// The words: centred in this width from this top, the wave and the ring
/// below the last line, so the projection grows down as lines come.
const TEXT_W: f32 = 480.0;
const TEXT_PX: f32 = 31.0;
const TEXT_LINE: f32 = 40.0;
const TEXT_TOP: f32 = 168.0;
const MAX_LINES: u32 = 5;
/// The waveform at the column's base, this far below the last line.
const WAVE_GAP: f32 = 46.0;
const WAVE_X0: f32 = 136.0;
const WAVE_X1: f32 = 464.0;
const WAVE_HALF: f32 = 30.0;
const BARS: usize = 49;
/// Seconds of audio between neighbouring bars, out from the middle.
const BAR_SECS: f64 = 0.03;
/// Levels under this are room tone and draw flat.
const WAVE_GATE: f32 = 0.04;
/// The ring pedestal, this far below the wave, and the status on it.
const RING_GAP: f32 = 82.0;
const RING_PAD: f32 = 40.0;
const RING_RX: f32 = 214.0;
const RING_RY: f32 = 26.0;
const STATUS_PX: f32 = 13.5;
/// Scan lines in the column: their pitch and how fast they drift down.
const SCAN_PITCH: f32 = 5.0;
const SCAN_SPEED: f32 = 6.0;
/// Idle: a brighter scan line passes down the column this often, and the
/// older lines float a pixel up and down, each a half period apart.
const SWEEP_SECS: f64 = 6.0;
const SWEEP_ALPHA: f32 = 0.12;
const FLOAT_SECS: f64 = 3.0;
const FLOAT_PX: f32 = 1.0;
/// Seconds a line that left the reading place waits before it floats.
const FLOAT_AFTER_SECS: f64 = 2.0;
const SETTLE_SECS: f64 = 0.35;

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn face(look: &Look) -> u32 {
    look.fonts
        .system("Google Sans Flex:medium")
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
        .tracking(-0.4 * k)
        .centered()
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

/// The wave's middle in design units with `lines` lines of words.
fn wave_y(lines: u32) -> f32 {
    TEXT_TOP + lines as f32 * TEXT_LINE + WAVE_GAP
}

fn ring_y(lines: u32) -> f32 {
    wave_y(lines) + RING_GAP
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    let h = ring_y(MAX_LINES) + RING_RY + RING_PAD;
    Size::new((DESIGN_W * k).ceil(), (h * k).ceil())
}

pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    let lines = lines.clamp(1, MAX_LINES);
    (TEXT_TOP + (lines as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// Points round an ellipse.
fn ellipse(c: Point, rx: f32, ry: f32) -> Contour {
    let n = 96;
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Point::new(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect();
    Contour::polyline(&pts, true)
}

/// The light: the column with its scan lines, the ring pedestal and the
/// orb, in the surface's space.
#[derive(Clone, Copy)]
struct Light {
    k: f32,
    /// The ring's middle in design units.
    ring: f32,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    still: bool,
}

impl Light {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening | Mode::Finishing if !self.still => f64::INFINITY,
            _ => self.mode_at + SETTLE_SECS + 0.1,
        }
    }

    /// 1 for the full blue light, down to 0 for a cancel's grey.
    fn power(&self, t: f64) -> f32 {
        let change = if self.still {
            1.0
        } else {
            super::kit::ease_out((t - self.mode_at) / SETTLE_SECS)
        };
        match self.mode {
            Mode::Cancelled => 1.0 - change,
            Mode::Loading => 0.55,
            _ => 1.0,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let power = self.power(t);
        let tint = |c: u32| mix(hex(DIM), hex(c), power);
        let (x0, x1) = (r.x + BEAM_X0 * k, r.x + BEAM_X1 * k);
        let (top, ring) = (r.y + BEAM_TOP * k, r.y + self.ring * k);
        // The column: clear at the top, a faint blue towards the ring.
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(x0, top, x1 - x0, ring - top), 18.0 * k),
            Paint::linear(
                Point::new(0.0, top),
                tint(HALO).fade(0.03),
                Point::new(0.0, ring),
                tint(HALO).fade(0.16),
            ),
        ));
        // The column's top: a faint rim under the orb, like a glass cylinder.
        let cap = ellipse(
            Point::new((x0 + x1) / 2.0, top),
            (x1 - x0) / 2.0,
            CAP_RY * k,
        );
        p.fill([&cap], &StrokeStyle::solid(1.0, tint(HALO).fade(0.06)));
        p.stroke(
            &cap,
            &StrokeStyle::solid(1.0 * k, tint(ICE).fade(0.14 * power)),
        );
        // Scan lines drifting down the column, fainter higher up.
        let drift = if self.still {
            0.0
        } else {
            ((t - self.shown_at) as f32 * SCAN_SPEED * k).rem_euclid(SCAN_PITCH * k)
        };
        let mut y = top + drift;
        while y < ring - 4.0 * k {
            let u = (y - top) / (ring - top);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0 + 8.0 * k, y, x1 - x0 - 16.0 * k, 1.0), 0.0),
                Paint::solid(tint(ICE).fade(0.035 * u)),
            ));
            y += SCAN_PITCH * k;
        }
        // Idle: one brighter scan line passes down the column every few
        // seconds while the popup listens.
        if !self.still && self.mode == Mode::Listening {
            let u = ((t - self.shown_at) / SWEEP_SECS).fract() as f32;
            let y = top + u * (ring - top);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(x0 + 8.0 * k, y, x1 - x0 - 16.0 * k, 1.0), 0.0),
                Paint::solid(tint(ICE).fade(SWEEP_ALPHA)),
            ));
        }
        // The ring pedestal: a glowing ellipse, a soft fill and a wider halo.
        let c = Point::new(r.x + r.w / 2.0, ring);
        let shimmer = if self.still || self.mode != Mode::Listening {
            0.0
        } else {
            0.08 * ((t - self.shown_at) * std::f64::consts::TAU / 3.0).sin() as f32
        };
        p.fill(
            [&ellipse(c, RING_RX * k, RING_RY * k)],
            &StrokeStyle::solid(1.0, tint(BLUE).fade(0.26 * power)),
        );
        p.stroke(
            &ellipse(c, (RING_RX + 30.0) * k, (RING_RY + 9.0) * k),
            &StrokeStyle::solid(1.0 * k, tint(HALO).fade(0.25 * power)),
        );
        p.stroke(
            &ellipse(c, RING_RX * k, RING_RY * k),
            &StrokeStyle::solid(2.6 * k, tint(BLUE))
                .linear(
                    Point::new(c.x - RING_RX * k, 0.0),
                    tint(CYAN),
                    Point::new(c.x + RING_RX * k, 0.0),
                    tint(VIOLET),
                )
                .glow((8.0 + 30.0 * shimmer) * k),
        );
        // The microphone orb.
        let o = Point::new(c.x, r.y + ORB_Y * k);
        let rad = ORB_R * k;
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::glow(tint(HALO).fade(0.5 * power), 14.0 * k, 0.5),
        ));
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::solid(hex(ORB).fade(0.92)),
        ));
        p.stroke(
            &make::ring(o, rad),
            &StrokeStyle::solid(1.6 * k, tint(CYAN)).glow(5.0 * k),
        );
        p.stroke(
            &make::ring(o, rad * 0.78),
            &StrokeStyle::solid(1.0 * k, tint(BLUE).fade(0.6)),
        );
        let unit = rad / 24.0 * 0.9;
        let mic = Outline::new(&icons::mic(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(unit))
                .then(&Affine::translate(o.x, o.y)),
        );
        p.outline(&mic, &StrokeStyle::solid(1.8 * unit, tint(ICE)));
    }
}

/// The waveform at the column's base: bars mirrored out from the middle,
/// the newest audio in the middle, cyan to blue to violet.
#[derive(Clone)]
struct Wave {
    k: f32,
    levels: Levels,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
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
        let change = super::kit::ease_out((t - self.mode_at) / SETTLE_SECS);
        let (height, grey) = match self.mode {
            Mode::Loading => (0.0, 0.4),
            Mode::Finishing => (1.0 - 0.6 * change, 0.0),
            Mode::Done => (0.4 * (1.0 - change), 0.0),
            Mode::Cancelled => (1.0 - change, change),
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
            let taper = (1.0 - reach.powf(1.4)).max(0.0);
            let level = (raw * 2.2).min(1.0) * jitter * taper * height;
            let half = (level * WAVE_HALF * k).max(1.0 * k);
            let x = r.x + i as f32 * pitch;
            let u = i as f32 / (BARS - 1) as f32;
            let c = if u < 0.5 {
                mix(hex(CYAN), hex(BLUE), 2.0 * u)
            } else {
                mix(hex(BLUE), hex(VIOLET), 2.0 * u - 1.0)
            };
            let c = mix(c, hex(DIM), grey).fade(1.0 - 0.4 * reach);
            if half <= 1.2 * k {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.1 * k,
                    Paint::solid(c),
                ));
                continue;
            }
            let bar = Rect::new(x - 1.2 * k, cy - half, 2.4 * k, 2.0 * half);
            p.shape(ShapeInstance::capsule(
                bar,
                Paint::glow(c.fade(0.45), 4.0 * k, 0.4),
            ));
            p.shape(ShapeInstance::capsule(bar, Paint::solid(c)));
        }
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

/// MO-83 on a centred line, as on particles: the letters of a word drop
/// 2 px together and bounce, with no sideways nudge, drift or stagger. A
/// centred line moves as it grows, and those on top of it loosened the
/// reading line's spacing past its limit.
fn letters() -> wild_motion::LetterPhysics {
    wild_motion::LetterPhysics {
        nudge: 0.0,
        drift: 0.0,
        stagger: 0.0,
        drop: 2.0,
        ..wild_motion::LetterPhysics::default()
    }
}

/// The words' colour, for effects that draw letters of their own.
pub(crate) fn text_color() -> Color {
    hex(TEXT)
}

/// The look's transcript for the outline features.
fn hologram_paths(look: &Look) -> PathsText {
    PathsText {
        style: look.text_style(),
        width: look.text_width(),
        line: TEXT_LINE * scale(look),
        ink: hex(TEXT),
        nib: hex(CYAN),
    }
}

impl Popup {
    pub(crate) fn hologram_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.hologram_projection())
    }

    fn hologram_projection(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let size = surface(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let cancelled = mode == Mode::Cancelled;
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -8.0)
        };
        let lines = self.lines.clamp(1, MAX_LINES);
        let light = Light {
            k,
            ring: ring_y(lines),
            shown_at: self.shown_at,
            mode,
            mode_at,
            still: self.still(),
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
        };
        let wave_until = wave.until(self.extra.now);
        let text_top = TEXT_TOP * k;
        let text_x = (DESIGN_W - TEXT_W) / 2.0 * k;
        Element::layer("hologram")
            .size(size.w, size.h)
            .presence(Transition::fade().offset(0.0, 8.0), exit)
            .child(at(
                Element::canvas("light", light.until(), move |t, r, p| light.draw(t, r, p))
                    .size(size.w, size.h),
                0.0,
                0.0,
            ))
            .child(at(
                Element::canvas("wave", wave_until, move |t, r, p| wave.draw(t, r, p))
                    .size((WAVE_X1 - WAVE_X0) * k, 2.0 * WAVE_HALF * k),
                WAVE_X0 * k,
                (wave_y(lines) - WAVE_HALF) * k,
            ))
            .children(self.limit_line(
                Point::new(text_x, text_top),
                size,
                hex(TEXT).fade(0.55),
                hex(CYAN),
            ))
            .child(self.hologram_text_clip(mode, lines, k, text_x))
            .child(at(
                self.hologram_status(mode, mode_at, k),
                (DESIGN_W / 2.0 - RING_RX) * k,
                (ring_y(lines) - 10.0) * k,
            ))
    }

    /// The words, clipped to the column under the orb, so a line that
    /// leaves on an overflow fades out inside the light.
    fn hologram_text_clip(&self, mode: Mode, lines: u32, k: f32, text_x: f32) -> Element<Msg> {
        let m = 10.0 * k;
        let top = BEAM_TOP + 6.0;
        let y = (TEXT_TOP - top) * k;
        let h = y + lines as f32 * TEXT_LINE * k + m;
        let clip = Element::layer("text-clip")
            .size(self.look.text_width() + 2.0 * m, h)
            .clip()
            .child(at(self.hologram_text(mode, lines), m, y));
        at(clip, text_x - m, top * k)
    }

    fn hologram_text(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let stacking = l.on(wild_space::DEPTH_STACK);
        let paths = hologram_paths(l);
        let words = if stacking {
            self.hologram_stack(mode, lines)
        } else {
            let words = self.hologram_words(&self.shown, mode).word_enter(
                self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, -8.0 * k)),
            );
            self.typo_words_in(self.hologram_letters(words), &paths)
        };
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            hex(TEXT),
            hex(CYAN),
        );
        let editing = self.edit_view.is_some();
        let text = Element::column("text")
            .width(l.text_width())
            .height(lines as f32 * TEXT_LINE * k)
            .child(words)
            .children(self.paths_overlays(&paths).into_iter().filter(|_| !editing))
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink {
                    text: hex(TEXT),
                    mark: hex(CYAN),
                    faint: hex(DIM),
                },
                endings::Cancel::Unthread,
            ));
        // MO-87: a revised word flows into its correction over the words.
        let text = self.letter_fx_layers(text, l.text_width());
        // The depth stack lays its lines out on its own and carries the
        // ghost tail on its reading line.
        let text = if stacking {
            text
        } else {
            self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), hex(TEXT))
        };
        self.with_hearing(text, &l.text_style(), l.text_width(), hex(TEXT), hex(CYAN))
    }

    /// The live text of `words` in the look's projected type.
    fn hologram_words(&self, words: &[Word], mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.4 } else { 1.0 };
        Element::live_text("words", words)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(TEXT))
            .text_halo(hex(HALO).fade(0.55), 9.0 * scale(l))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
    }

    /// The letter effects on the reading line's live text: the liquid
    /// morph's swaps, and letter physics tuned for a centred line.
    fn hologram_letters(&self, words: Element<Msg>) -> Element<Msg> {
        let words = if self.look.on(wild_motion::LETTER_PHYSICS) {
            // The letters' drop moves the glyphs: a revision swaps them at
            // once, as on particles.
            words.word_revise(vo::scene::flow::Revise::Swap)
        } else {
            words
        };
        self.letter_fx_words(words)
            .wild_motion_with(self, letters())
    }

    /// Whether the older lines float: while the popup listens, not under
    /// reduced motion, and only with older lines on the depth stack's
    /// sheets to move. The flat text has one live text for all its lines,
    /// so its older lines hold still.
    fn hologram_floats(&self) -> bool {
        self.look.theme == OsdTheme::Hologram
            && self.look.on(wild_space::DEPTH_STACK)
            && self.lines > 1
            && !self.closing
            && !self.still()
            && self.kit_mode() == Mode::Listening
    }

    /// Idle: how far the line `d` places behind the reading line floats now,
    /// a pixel up or down, each line a half period after the one in front.
    /// The reading line never moves.
    fn hologram_float(&self, d: usize) -> f32 {
        if d == 0 || !self.hologram_floats() {
            return 0.0;
        }
        // The line that just left the reading place settles first.
        let settling = self
            .jarvis
            .hold
            .as_ref()
            .is_some_and(|h| self.extra.now - h.since < FLOAT_AFTER_SECS);
        if d == 1 && settling {
            return 0.0;
        }
        let half = FLOAT_SECS / 2.0;
        let step = ((self.extra.now - self.shown_at) / half).floor() as i64;
        let up = (step + d as i64) % 2 == 0;
        let k = scale(&self.look);
        if up {
            -FLOAT_PX * k
        } else {
            FLOAT_PX * k
        }
    }

    /// Wakes the view at each half period while the older lines float.
    pub(crate) fn arm_hologram_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.extra.tick.is_some() || !self.hologram_floats() {
            return;
        }
        let half = FLOAT_SECS / 2.0;
        let next = half - (cx.t() - self.shown_at).rem_euclid(half);
        self.extra.tick = Some(cx.after(next + 0.01, Msg::Tick));
    }

    /// MO-86: the transcript as a depth stack. The reading line lies flat
    /// at the bottom row; older lines recede up and back on sheets of blue
    /// glass, dimmer and softer with each place, and float while idle.
    fn hologram_stack(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let style = l.text_style();
        let w = l.text_width();
        let line_h = style.line_height;
        let reduced = self.wild_reduced();
        // A little wider than the text, so a full line sits inside its sheet.
        let sheet = Size::new(w + 12.0 * k, line_h + 8.0 * k);
        let stack = DepthStack {
            step_z: 90.0 * k,
            step_rise: line_h * 0.82,
            step_x: 0.0,
            sheet: Some(Sheet {
                fill: hex(ORB).fade(0.45),
                edge: hex(CYAN).fade(0.35),
                edge_width: 1.0,
                radius: 6.0 * k,
                pad: (0.0, 4.0 * k),
            }),
            ..DepthStack::default()
        };
        let all = &self.jarvis.lines;
        let shown = &all[all.len().saturating_sub(lines as usize)..];
        let keyed: Vec<(vo::scene::Key, &(Vec<Word>, f32))> = shown
            .iter()
            .map(|line| {
                let first = line.0.first().map_or(0, |w| w.id);
                (wild_space::depth_stack_key(first), line)
            })
            .collect();
        let tail = |room: f32| model_signals::TailText {
            style,
            width: room,
            rows: 1,
        };
        let content = |line: &&(Vec<Word>, f32), d: usize, p: &Place| {
            let (words, width) = *line;
            let lw = width.ceil() + 2.0;
            // A word lands in place at full ink: the sheet of a new line
            // already rises from 70 % ink (as on jarvis).
            let live = self
                .hologram_words(words, mode)
                .text_blur(p.blur)
                .motion(Prop::TextFx, stack.motion(d, reduced))
                .size(lw, line_h);
            let live = if d == 0 {
                self.typo_words(self.hologram_letters(live))
            } else {
                live
            };
            let live = live
                .word_enter(Transition::NONE)
                .word_change(Transition::fade().opacity(0.7));
            let line_e = Element::layer("line").size(lw, line_h).child(live);
            let line_e = if d == 0 {
                self.signal_marks_in(
                    line_e,
                    words,
                    &tail((sheet.w + lw) / 2.0),
                    (0.0, 0.0),
                    hex(TEXT),
                )
            } else {
                line_e
            };
            at(
                line_e,
                ((sheet.w - lw) / 2.0).max(0.0),
                4.0 * k + self.hologram_float(d),
            )
            .motion(Prop::Position, Spring::SOFT)
        };
        let h = lines.max(1) as f32 * line_h;
        let x = (w - sheet.w) / 2.0;
        let view = at(
            stack.view_at(
                "depth-stack",
                Size::new(sheet.w, h),
                sheet,
                &keyed,
                content,
                reduced,
                self.jarvis.hold.as_ref(),
                self.extra.now,
            ),
            x,
            0.0,
        );
        let stack_box = Element::column("stack-box").size(w, h).child(view);
        if !keyed.is_empty() {
            return stack_box;
        }
        // No line yet: the forming cue where the first line lands.
        self.signal_marks_in(stack_box, &[], &tail(w), (0.0, h - line_h), hex(TEXT))
    }

    /// The status, centred on the ring: a dot, the state word and the
    /// detail or hint.
    fn hologram_status(&self, mode: Mode, mode_at: f64, k: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(ui(l), STATUS_PX * k, 20.0 * k);
        let status = self.status();
        let (state, detail, dot) = match mode {
            Mode::Done => {
                let detail = self
                    .kit_target()
                    .map(|a| format!("into {a}"))
                    .unwrap_or_default();
                (self.delivery_word().to_string(), detail, hex(GREEN))
            }
            Mode::Cancelled => (
                "Cancelled".to_string(),
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
                    &StrokeStyle::solid(1.3 * k, hex(ICE)),
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
            .width(2.0 * RING_RX * k)
            .height(20.0 * k)
            .align_items(AlignItems::CENTER)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
            .child(
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(hex(ICE))
                    .margin_edges(0.0, 0.0, 0.0, 9.0 * k)
                    .motion(Prop::Position, Motion::Snap),
            )
            .child(
                Element::live_text(format!("detail-{state}"), &words)
                    .text_style(style)
                    .color(hex(HINT))
                    .height(20.0 * k)
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
                .width(80.0 * k)
                .margin_edges(0.0, 0.0, 0.0, 12.0 * k),
            );
        }
        row
    }
}
