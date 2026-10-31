//! The `river` look of the vo popup, after concept 24, the river of light:
//! a wide, short strip of dark navy glass with a thin blue rim. A ribbon of
//! glowing strands runs from the microphone across the top and swings with
//! the voice, the words sit under it in a clean grotesk with the newest word
//! lit blue to violet, and a status row runs along the bottom.
//!
//! While the final text is made the ribbon calms; when it lands the ribbon
//! flattens into one bright hairline that ends in an arrow, the hand-off to
//! the app. A cancel lays the ribbon flat in grey and dims the words. Under
//! reduced motion the ribbon is drawn still.

use super::kit::{at, hex, mix, Mode};
use super::*;

use super::glyph_paths::PathsText;
use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point};
use vo::scene::levels::{Levels as LevelTrack, Stops};
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo::scene::ribbon::Ribbon;
use vo::scene::{MarkShape, WordMark};

/// Colours sampled from the redesign.
const BODY_TOP: u32 = 0x0d1838;
const BODY_BOTTOM: u32 = 0x070d24;
const RIM: u32 = 0x6f9bff;
const CYAN: u32 = 0x3fc8ff;
const BLUE: u32 = 0x3b6bff;
const VIOLET: u32 = 0xa45cff;
const ICE: u32 = 0xeef4ff;
const INK: u32 = 0xe9eefc;
const HINT: u32 = 0x8ea6e6;
const DIM: u32 = 0x6b7591;
const GREEN: u32 = 0x39d98a;
const NEWEST_LEFT: u32 = 0x8fd8ff;
const NEWEST_MID: u32 = 0x6f8dff;
const NEWEST_RIGHT: u32 = 0xb48bff;

/// The strip's width in the redesign's units at `width_px = 600`; the look
/// scales with the popup's width.
const DESIGN_W: f32 = 760.0;
const BASE_W: f32 = 600.0;
const RADIUS: f32 = 18.0;
/// The microphone's ring, and the ribbon's box to its right.
const MIC_X: f32 = 60.0;
const MIC_Y: f32 = 58.0;
const MIC_R: f32 = 24.0;
const RIVER_X0: f32 = 90.0;
const RIVER_X1: f32 = 736.0;
const RIVER_H: f32 = 70.0;
/// The words: left edge, top and line pitch.
const TEXT_X: f32 = 100.0;
const TEXT_RIGHT: f32 = 28.0;
const TEXT_TOP: f32 = 92.0;
const TEXT_PX: f32 = 25.0;
const TEXT_LINE: f32 = 32.0;
const MAX_LINES: u32 = 2;
/// The status row under the words.
const STATUS_GAP: f32 = 12.0;
const STATUS_H: f32 = 18.0;
const STATUS_X: f32 = 28.0;
const PAD_BOTTOM: f32 = 16.0;
/// The hand-off: the hairline and its arrow, and how long the ribbon takes
/// to flatten into it.
const SETTLE_SECS: f32 = 0.5;
const ARROW: f32 = 7.0;
/// Idle: specks of light that drift along the river, at most this bright.
const SPECKS: u32 = 14;
const SPECK_SPEED: f32 = 30.0;
const SPECK_ALPHA: f32 = 0.4;

/// R2-24-a: the stroke-write pen writes the newest word from 70 % ink to
/// full (`Look::on` turns `stroke-write` on with it), over a soft ink bleed
/// under the word that fades in `BLEED_SECS`.
pub(crate) const INK_REVEAL: &str = "typewriter-ink-reveal";
const BLEED_SECS: f64 = 0.45;
const BLEED_ALPHA: f32 = 0.14;
/// R2-24-b: an inner rim that breathes out towards the edge with the voice,
/// `BREATH_PX` at most, and rings that spread from the microphone and fade
/// before the words, one started every `RIPPLE_EVERY` seconds while there
/// is voice.
pub(crate) const GLASS_PANEL: &str = "breathing-glass-panel";
const BREATH_PX: f32 = 3.0;
const RIM_ALPHA: f32 = 0.2;
const RIPPLE_EVERY: f64 = 0.45;
const RIPPLE_LIFE: f64 = 1.6;
const RIPPLE_REACH: f32 = 150.0;
const RIPPLE_ALPHA: f32 = 0.16;
/// R2-24-c: a ribbon of light behind the words that swings with the voice
/// and flattens into a thin still line over `FLAT_SECS` once you stop.
pub(crate) const AUDIO_RIBBON: &str = "audio-ribbon";
const RIBBON_ALPHA: f32 = 0.2;
const FLAT_SECS: f32 = 0.6;
/// How far under the last line's box the ribbon's line rests, in design
/// pixels: under the descenders, clear of the status row.
const RIBBON_DROP: f32 = 2.0;
// A new word starts at 70 % ink or more, and the ribbon stays at 25 % or less.
const _: () = assert!(
    glyph_paths::ARRIVAL_INK >= 0.7 && glyph_paths::PEN_FLOOR >= 0.7 && RIBBON_ALPHA <= 0.25
);

/// The voice's share of full swing at `level`, 0 to 1.
fn voice(level: f32) -> f32 {
    (level * 2.6).clamp(0.0, 1.0)
}

/// R2-24-a: the bleed's alpha `age` seconds after the word came.
fn bleed_alpha(age: f64) -> f32 {
    if !(0.0..BLEED_SECS).contains(&age) {
        return 0.0;
    }
    let u = (age / BLEED_SECS) as f32;
    BLEED_ALPHA * (1.0 - u) * (1.0 - u)
}

/// R2-24-b: how far in from the strip's edge the inner rim sits at voice
/// level `level`, in pixels before scaling: `BREATH_PX` quiet, 0 at full
/// voice.
fn rim_inset(level: f32) -> f32 {
    BREATH_PX * (1.0 - voice(level))
}

/// R2-24-b: the rings at `t`, each its radius past the microphone's ring
/// (design pixels) and alpha, for a voice level `at(s)` at time `s`. No
/// ring starts after `heard`, the newest audio, so all are gone by
/// `heard + RIPPLE_LIFE`.
fn ripples(t: f64, heard: f64, at: impl Fn(f64) -> f32) -> Vec<(f32, f32)> {
    let last = (t.min(heard) / RIPPLE_EVERY).floor() as i64;
    let first = ((t - RIPPLE_LIFE) / RIPPLE_EVERY).ceil() as i64;
    (first.max(0)..=last)
        .filter_map(|j| {
            let born = j as f64 * RIPPLE_EVERY;
            let u = ((t - born) / RIPPLE_LIFE) as f32;
            let a = RIPPLE_ALPHA * voice(at(born)) * (1.0 - u).powf(1.5);
            (a > 0.003).then_some((u * RIPPLE_REACH, a))
        })
        .collect()
}

/// How much the strip is scaled from the redesign.
fn scale(look: &Look) -> f32 {
    look.width / BASE_W
}

fn sans(look: &Look) -> u32 {
    look.fonts.system("Inter").unwrap_or(SANS)
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let k = scale(look);
    TextStyle::new(sans(look), TEXT_PX * k, TEXT_LINE * k).tracking(-0.3 * k)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    (DESIGN_W - TEXT_X - TEXT_RIGHT) * scale(look)
}

pub(crate) fn max_lines() -> u32 {
    MAX_LINES
}

/// The strip's height in the redesign's units with `lines` lines of words.
fn strip_h(lines: u32) -> f32 {
    TEXT_TOP + lines as f32 * TEXT_LINE + STATUS_GAP + STATUS_H + PAD_BOTTOM
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    Size::new((DESIGN_W * k).ceil(), (strip_h(MAX_LINES) * k).ceil())
}

/// The middle of the newest line below the surface's top, with `lines`
/// lines of words.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (TEXT_TOP + (lines.clamp(1, MAX_LINES) as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// The microphone in its glowing ring.
#[derive(Clone, Copy)]
struct Mic {
    k: f32,
    lit: f32,
}

impl Mic {
    fn draw(&self, r: vo::scene::geom::Rect, p: &mut Painter) {
        let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rad = MIC_R * self.k;
        let ring = mix(hex(DIM), hex(CYAN), self.lit);
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::glow(ring.fade(0.45 * self.lit), 10.0 * self.k, 0.5),
        ));
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::solid(hex(BODY_BOTTOM)),
        ));
        p.stroke(
            &make::ring(c, rad),
            &StrokeStyle::solid(1.6 * self.k, ring).glow(4.0 * self.k),
        );
        let unit = rad / 24.0 * 0.95;
        let mic = Outline::new(&icons::mic(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(unit))
                .then(&Affine::translate(c.x, c.y)),
        );
        p.outline(
            &mic,
            &StrokeStyle::solid(1.8 * unit, mix(hex(DIM), hex(ICE), self.lit)),
        );
    }
}

/// The hairline the ribbon becomes at the hand-off, with its arrow.
#[derive(Clone, Copy)]
struct Hairline {
    k: f32,
    at: f64,
    still: bool,
}

impl Hairline {
    fn until(&self) -> f64 {
        self.at + SETTLE_SECS as f64 + 0.1
    }

    fn draw(&self, t: f64, r: vo::scene::geom::Rect, p: &mut Painter) {
        let grow = if self.still {
            1.0
        } else {
            super::kit::ease_out((t - self.at) / SETTLE_SECS as f64)
        };
        if grow <= 0.0 {
            return;
        }
        let y = r.y + r.h / 2.0;
        let x1 = r.x + r.w * grow;
        let line = make::underline(r.x, x1, y);
        p.stroke(
            &line,
            &StrokeStyle::solid(1.4 * self.k, hex(ICE))
                .linear(
                    Point::new(r.x, y),
                    hex(BLUE),
                    Point::new(r.x + r.w, y),
                    hex(ICE),
                )
                .glow(5.0 * self.k),
        );
        let a = ARROW * self.k;
        let tip = Point::new(x1, y);
        for dy in [-1.0f32, 1.0] {
            let tail = Point::new(x1 - a, y + dy * a * 0.7);
            p.stroke(
                &vo::scene::path::Contour::polyline(&[tail, tip], false),
                &StrokeStyle::solid(1.4 * self.k, hex(ICE).fade(grow)).glow(4.0 * self.k),
            );
        }
    }
}

/// The look's transcript for the outline features: its own text style and
/// line pitch, inked in the look's colours.
fn river_paths(look: &Look) -> PathsText {
    PathsText {
        style: look.text_style(),
        width: look.text_width(),
        line: TEXT_LINE * scale(look),
        ink: hex(INK),
        nib: hex(CYAN),
    }
}

/// Idle specks of light drifting along the river.
#[derive(Clone, Copy)]
struct Specks {
    k: f32,
    shown_at: f64,
}

impl Specks {
    fn draw(&self, t: f64, r: vo::scene::geom::Rect, p: &mut Painter) {
        let k = self.k;
        let run = (t - self.shown_at) as f32 * SPECK_SPEED * k;
        let hash = |i: u32, s: u32| {
            let x = (i.wrapping_mul(0x9E37_79B9) ^ s).wrapping_mul(0x85EB_CA6B);
            ((x >> 16) & 0xFFFF) as f32 / 65535.0
        };
        let cy = r.y + r.h / 2.0;
        for i in 0..SPECKS {
            let x = (run + r.w * hash(i, 1)).rem_euclid(r.w);
            let u = x / r.w;
            // Faded in from the microphone and out at the far end.
            let ends = (u / 0.1).min((1.0 - u) / 0.1).clamp(0.0, 1.0);
            let y = cy
                + (hash(i, 2) - 0.5) * 0.5 * r.h
                + 4.0 * k * (x / (90.0 * k) + 6.0 * hash(i, 3)).sin();
            let a = SPECK_ALPHA * (0.4 + 0.6 * hash(i, 4)) * ends;
            if a <= 0.0 {
                continue;
            }
            let c = Point::new(r.x + x, y);
            let col = mix(hex(CYAN), hex(VIOLET), u);
            p.shape(ShapeInstance::circle(
                c,
                1.1 * k,
                Paint::glow(col.fade(a), 3.0 * k, 0.5),
            ));
            p.shape(ShapeInstance::circle(
                c,
                0.9 * k,
                Paint::solid(hex(ICE).fade(a)),
            ));
        }
    }
}

impl Popup {
    pub(crate) fn river_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.river_strip())
    }

    /// The whole strip, as tall as its lines.
    fn river_strip(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let lines = self.lines.clamp(1, MAX_LINES);
        let (w, h) = ((DESIGN_W * k).ceil(), strip_h(lines) * k);
        let cancelled = mode == Mode::Cancelled;
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let lit = match mode {
            Mode::Loading => 0.45,
            Mode::Cancelled => 0.0,
            _ => 1.0,
        };
        let mic = Mic { k, lit };
        let side = 2.0 * (MIC_R + 12.0) * k;
        Element::panel("strip")
            .width(w)
            .height(h)
            .radius(RADIUS * k)
            .fill_gradient(Color::hex(BODY_TOP, 0.94), Color::hex(BODY_BOTTOM, 0.94))
            .border(Color::hex(RIM, 0.55), self.exhale_width(1.0))
            .rim(Color::hex(RIM, 0.45), 1.0, 0.18, -0.4, 0.0)
            .presence(Transition::fade().offset(0.0, 6.0), exit)
            .motion(Prop::Size, Motion::Snap)
            .child(self.river_ribbon(mode, mode_at, k))
            .children(self.river_panel(mode, Size::new(w, h), k))
            .children(self.river_specks(mode, k))
            .child(
                at(
                    Element::canvas("mic", self.shown_at, move |_, r, p| mic.draw(r, p)),
                    MIC_X * k - side / 2.0,
                    MIC_Y * k - side / 2.0,
                )
                .size(side, side),
            )
            .children(self.river_sculpture(mode, Size::new(w, h), k))
            .children(self.river_lattice(mode, k))
            .children(self.limit_line(
                Point::new(TEXT_X * k, TEXT_TOP * k),
                Size::new(w, h),
                hex(INK).fade(0.55),
                hex(CYAN),
            ))
            .children(self.river_audio_ribbon(mode, lines, k))
            .children(self.river_bleed(mode, lines, k))
            .child(self.river_text_clip(mode, lines, k))
            .child(at(
                self.river_status(mode, mode_at, k, w),
                STATUS_X * k,
                (TEXT_TOP + lines as f32 * TEXT_LINE + STATUS_GAP) * k,
            ))
    }

    /// The ribbon of light over the words, or the hand-off's hairline.
    fn river_ribbon(&self, mode: Mode, mode_at: f64, k: f32) -> Element<Msg> {
        let still = self.still();
        let rect = (
            RIVER_X0 * k,
            (MIC_Y - RIVER_H / 2.0) * k,
            (RIVER_X1 - RIVER_X0) * k,
            RIVER_H * k,
        );
        if mode == Mode::Done {
            let line = Hairline {
                k,
                at: mode_at,
                still,
            };
            return at(
                Element::canvas("hairline", line.until(), move |t, r, p| line.draw(t, r, p)),
                rect.0,
                rect.1,
            )
            .size(rect.2, rect.3);
        }
        let levels = self.exhale_levels(&self.kit_levels().levels);
        let (cyan, blue, violet) = (hex(CYAN), hex(BLUE), hex(VIOLET));
        let mut ribbon = Ribbon {
            levels: LevelTrack {
                levels,
                newest_at: self.newest_at,
                rate: FRAME_HZ as f32,
                ..LevelTrack::default()
            },
            left: Stops::new(&[(0.0, cyan), (1.0, blue)]),
            right: Stops::new(&[(0.0, violet), (0.5, blue), (1.0, cyan)]),
            core: Some(hex(ICE).fade(0.9)),
            settle_secs: SETTLE_SECS,
            // Thin strands that swing widest near the microphone, as drawn.
            strands: 6,
            cycles: 1.8,
            gain: 1.0,
            floor: 0.3,
            belly: 0.32,
            width: 1.6 * k,
            alpha: 0.8,
            glow: 6.0 * k,
            // One stroke per strand: glows that overlap where runs meet
            // drew bright beads along the ribbon.
            run: 1024,
            ..Ribbon::default()
        };
        match mode {
            Mode::Loading => {
                ribbon.alpha = 0.35;
                ribbon.core = None;
                ribbon.gain = ribbon.floor;
            }
            Mode::Finishing => {
                ribbon.gain *= 0.5;
                ribbon.speed *= 0.5;
            }
            Mode::Cancelled => {
                let grey = mix(hex(DIM), hex(BODY_TOP), 0.2);
                ribbon.left = Stops::new(&[(0.0, grey), (1.0, grey)]);
                ribbon.right = Stops::new(&[(0.0, grey), (1.0, grey)]);
                ribbon.core = None;
                ribbon.alpha = 0.5;
                ribbon.settle = Some(mode_at);
            }
            _ => {}
        }
        if still {
            // No material motion: the strands stand at their resting swing.
            ribbon.speed = 0.0;
            ribbon.levels = LevelTrack {
                newest_at: self.shown_at,
                ..LevelTrack::default()
            };
            if mode == Mode::Cancelled {
                ribbon.settle_secs = 0.001;
            }
        }
        at(Element::ribbon("ribbon", ribbon), rect.0, rect.1)
            .size(rect.2, rect.3)
            .presence(Transition::fade(), Transition::fade())
    }

    /// The words, clipped to their box: a line that leaves on an overflow
    /// stays out of the ribbon above. The margin keeps the pen and the word
    /// entry whole at the sides and below.
    fn river_text_clip(&self, mode: Mode, lines: u32, k: f32) -> Element<Msg> {
        let m = 8.0 * k;
        let size = Size::new(
            self.look.text_width() + 2.0 * m,
            lines as f32 * TEXT_LINE * k + m,
        );
        let clip = Element::layer("text-clip")
            .size(size.w, size.h)
            .clip()
            .child(at(self.river_text(mode), m, 0.0));
        at(clip, TEXT_X * k - m, TEXT_TOP * k)
    }

    fn river_text(&self, mode: Mode) -> Element<Msg> {
        let l = &self.look;
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(INK))
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0)))
            // The newest word runs from pale blue through blue to violet.
            .current_word(
                self.shown
                    .last()
                    .map(|w| w.id)
                    .filter(|_| mode == Mode::Listening),
            )
            .word_mark(
                WordMark::new(MarkShape::Underline, Color::TRANSPARENT).accent_gradient(
                    hex(NEWEST_LEFT),
                    hex(NEWEST_MID),
                    hex(NEWEST_RIGHT),
                ),
            )
            .wild_motion(self);
        // MO-24 and the pen's letter fills (stroke-write) follow this look's
        // own text style and line pitch.
        let paths = river_paths(l);
        let words = self.typo_words_in(words, &paths);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            hex(INK),
            hex(CYAN),
        );
        let editing = self.edit_view.is_some();
        let text = Element::column("text")
            .width(l.text_width())
            .height(self.lines.clamp(1, MAX_LINES) as f32 * l.text_style().line_height)
            .child(words)
            // MO-80: the pen over the words while they are being written.
            .children(self.paths_overlays(&paths).into_iter().filter(|_| !editing))
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink {
                    text: hex(INK),
                    mark: hex(CYAN),
                    faint: hex(DIM),
                },
                endings::Cancel::Unthread,
            ));
        let text = self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), hex(INK));
        self.with_hearing(text, &l.text_style(), l.text_width(), hex(INK), hex(CYAN))
    }

    /// R2-24-b: the glass panel's inner rim, breathing with the voice, and
    /// the rings from the microphone, inside the strip. Still under reduced
    /// motion: the rim at rest and no rings.
    fn river_panel(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        if !self.look.on(GLASS_PANEL) || mode == Mode::Cancelled || self.edit_view.is_some() {
            return None;
        }
        let still = self.still();
        let levels = self.kit_levels();
        let mic = Point::new(MIC_X * k, MIC_Y * k);
        let fade = (TEXT_X * k, (TEXT_X + 110.0) * k);
        let heard = self.newest_at;
        let until = if still {
            self.shown_at
        } else {
            heard + RIPPLE_LIFE
        };
        let canvas = Element::canvas("panel", until, move |t, r, p| {
            let level = if still {
                0.0
            } else {
                (levels.at(t) + levels.at(t - 0.05) + levels.at(t - 0.1)) / 3.0
            };
            let i = (rim_inset(level) + 1.0) * k;
            let rim = Rect::new(r.x + i, r.y + i, r.w - 2.0 * i, r.h - 2.0 * i);
            let c = Color::hex(RIM, RIM_ALPHA);
            p.stroke(
                &make::panel(rim.x, rim.y, rim.w, rim.h, (RADIUS * k - i).max(0.0)),
                &StrokeStyle::solid(1.0 * k, c).glow(3.0 * k),
            );
            if still {
                return;
            }
            let centre = Point::new(r.x + mic.x, r.y + mic.y);
            for (reach, a) in ripples(t, heard, |s| levels.at(s)) {
                let rx = (MIC_R + 4.0 + reach) * k;
                let ry = 0.55 * rx;
                let pts: Vec<Point> = (0..72)
                    .map(|j| {
                        let ang = j as f32 / 72.0 * std::f32::consts::TAU;
                        Point::new(centre.x + rx * ang.cos(), centre.y + ry * ang.sin())
                    })
                    .collect();
                let c = Color::hex(CYAN, a);
                p.stroke(
                    &Contour::polyline(&pts, true),
                    &StrokeStyle::solid(1.2 * k, c)
                        .linear(
                            Point::new(r.x + fade.0, 0.0),
                            c,
                            Point::new(r.x + fade.1, 0.0),
                            c.fade(0.0),
                        )
                        .glow(3.0 * k),
                );
            }
        });
        let layer = Element::layer("panel-fx")
            .size(size.w, size.h)
            .radius(RADIUS * k)
            .clip()
            .presence(Transition::fade(), Transition::fade())
            .child(at(canvas.size(size.w, size.h), 0.0, 0.0));
        Some(at(layer, 0.0, 0.0))
    }

    /// R2-24-c: the ribbon behind the words. It swings with the voice and
    /// flattens into a thin still line over `FLAT_SECS` from the stop; under
    /// reduced motion it is that line, as bright as the voice.
    fn river_audio_ribbon(&self, mode: Mode, lines: u32, k: f32) -> Option<Element<Msg>> {
        if !self.look.on(AUDIO_RIBBON) || self.edit_view.is_some() {
            return None;
        }
        let still = self.still();
        // The ribbon rests on a line just under the words' descenders and
        // rises behind them; the half that would swing below is clipped, so
        // it never reaches the status row.
        let rest = (TEXT_TOP + lines as f32 * TEXT_LINE + RIBBON_DROP) * k;
        let half = 0.6 * lines as f32 * TEXT_LINE * k;
        let w = (RIVER_X1 - TEXT_X) * k;
        let stop = match mode {
            Mode::Listening | Mode::Loading => None,
            _ => Some(self.kit_mode_at(Mode::Finishing).min(self.extra.now)),
        };
        let level = self.kit_levels().at(self.extra.now);
        let mut ribbon = Ribbon {
            levels: LevelTrack {
                levels: self.exhale_levels(&self.kit_levels().levels),
                newest_at: self.newest_at,
                rate: FRAME_HZ as f32,
                ..LevelTrack::default()
            },
            left: Stops::new(&[(0.0, hex(CYAN)), (1.0, hex(BLUE))]),
            right: Stops::new(&[(0.0, hex(VIOLET)), (0.5, hex(BLUE)), (1.0, hex(CYAN))]),
            core: None,
            strands: 3,
            cycles: 1.4,
            gain: 1.0,
            floor: 0.45,
            belly: 0.45,
            width: 1.2 * k,
            alpha: RIBBON_ALPHA,
            glow: 4.0 * k,
            settle: stop,
            settle_secs: FLAT_SECS,
            run: 1024,
            ..Ribbon::default()
        };
        if mode == Mode::Cancelled {
            ribbon.alpha = 0.5 * RIBBON_ALPHA;
        }
        if still {
            ribbon.speed = 0.0;
            ribbon.levels = LevelTrack {
                newest_at: self.shown_at,
                ..LevelTrack::default()
            };
            ribbon.settle = Some(self.shown_at);
            ribbon.settle_secs = 0.001;
            ribbon.alpha = RIBBON_ALPHA * (0.35 + 0.65 * voice(level));
        }
        let clip = Element::layer("audio-ribbon-clip")
            .size(w, half + 2.0 * k)
            .clip()
            .presence(Transition::fade(), Transition::fade())
            .child(at(Element::ribbon("audio-ribbon", ribbon), 0.0, 0.0).size(w, 2.0 * half));
        Some(at(clip, TEXT_X * k, rest - half))
    }

    /// R2-24-a: a soft ink bleed under the newest word, fading as the pen
    /// writes it. None under reduced motion.
    fn river_bleed(&self, mode: Mode, lines: u32, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(INK_REVEAL) || mode != Mode::Listening || self.still() || self.edit_view.is_some()
        {
            return None;
        }
        let style = l.text_style();
        let rows = self.wrap_lines(&self.shown, &style, l.text_width());
        let last = rows.last()?;
        let newest = last.last()?;
        let born = self.heard.word(newest.id)?.born;
        if self.extra.now - born >= BLEED_SECS {
            return None;
        }
        let (boxes, _) = sounding::word_boxes(&self.measure, last, &style, None);
        let b = boxes.iter().find(|b| b.id == newest.id)?;
        let y = (TEXT_TOP + (lines as f32 - 0.5) * TEXT_LINE) * k + 0.15 * style.size;
        let (x, w) = (TEXT_X * k + b.rect.x, b.rect.w);
        let rad = 0.42 * style.size;
        let canvas = Element::canvas("bleed", born + BLEED_SECS, move |t, r, p| {
            let a = bleed_alpha(t - born);
            if a <= 0.002 {
                return;
            }
            let n = ((w / rad).ceil() as usize).max(1);
            for i in 0..=n {
                let cx = x + w * i as f32 / n as f32;
                p.shape(ShapeInstance::circle(
                    Point::new(r.x + cx, r.y + y),
                    rad,
                    Paint::glow(hex(BLUE).fade(a), rad, 0.5),
                ));
            }
        });
        let size = surface(l);
        Some(at(canvas.size(size.w, size.h), 0.0, 0.0))
    }

    /// Idle: light specks flow along the river at 30 px a second while the
    /// popup listens, under the words. None under reduced motion.
    fn river_specks(&self, mode: Mode, k: f32) -> Option<Element<Msg>> {
        if mode != Mode::Listening || self.still() || self.closing {
            return None;
        }
        let specks = Specks {
            k,
            shown_at: self.shown_at,
        };
        let (x, y) = (RIVER_X0 * k, (MIC_Y - RIVER_H / 2.0) * k);
        let size = Size::new((RIVER_X1 - RIVER_X0) * k, RIVER_H * k);
        Some(
            at(
                Element::canvas("specks", f64::INFINITY, move |t, r, p| specks.draw(t, r, p)),
                x,
                y,
            )
            .size(size.w, size.h)
            .presence(Transition::fade(), Transition::fade()),
        )
    }

    /// MO-88: with `sound-sculpture`, letter bodies behind the words, as
    /// deep as each word was loud; they lie flat once the dictation stops.
    fn river_sculpture(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(wild_space::SOUND_SCULPTURE) || self.edit_view.is_some() {
            return None;
        }
        let style = l.text_style();
        let lines = self.wrap_lines(&self.shown, &style, l.text_width());
        let skip = lines.len().saturating_sub(MAX_LINES as usize);
        let rows: Vec<(Vec<Word>, Point)> = lines
            .into_iter()
            .skip(skip)
            .enumerate()
            .map(|(i, ws)| {
                (
                    ws,
                    Point::new(TEXT_X * k, (TEXT_TOP + i as f32 * TEXT_LINE) * k),
                )
            })
            .collect();
        let done = mode != Mode::Listening || self.still();
        self.sculpture_element(&rows, &style, size, Spring::POP, done)
    }

    /// MO-85: with `kinetic-lattice`, the lattice under the words, its
    /// newest gutter the gap down to the status row.
    fn river_lattice(&self, mode: Mode, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(wild_space::KINETIC_LATTICE) || self.edit_view.is_some() {
            return None;
        }
        let set = wild_space::LatticeSet {
            preset: vo::scene::lattice::LatticeStyle::Light,
            color: hex(HINT).fade(0.5),
            accent: hex(CYAN),
            scale: 0.9 * k,
            label: Some(TextStyle::new(sans(l), 9.0 * k, 11.0 * k)),
            gutter: (STATUS_GAP - 3.0) * k,
            opacity: if mode == Mode::Cancelled { 0.35 } else { 1.0 },
        };
        let style = l.text_style();
        let e = self.lattice_lines("river", &self.shown, &style, l.text_width(), &set)?;
        Some(at(e, TEXT_X * k, TEXT_TOP * k))
    }

    /// The status row: a dot, the state word, and the detail or hint.
    fn river_status(&self, mode: Mode, mode_at: f64, k: f32, w: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(sans(l), 13.0 * k, STATUS_H * k);
        let hint = TextStyle::new(sans(l), 12.5 * k, STATUS_H * k);
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
        let breathe = mode == Mode::Listening && !self.still();
        let since = self.shown_at;
        let mark = Element::canvas(
            "dot",
            if breathe { f64::INFINITY } else { mode_at },
            move |t, r, p| {
                let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
                let a = if breathe {
                    0.75 + 0.25 * ((t - since) * std::f64::consts::TAU * 0.5).cos() as f32
                } else {
                    1.0
                };
                p.shape(ShapeInstance::circle(
                    c,
                    3.6 * k,
                    Paint::glow(dot.fade(0.5 * a), 4.0 * k, 0.5),
                ));
                p.shape(ShapeInstance::circle(c, 3.6 * k, Paint::solid(dot.fade(a))));
            },
        )
        .size(10.0 * k, 10.0 * k);
        let words = super::kit::detail_words(&state, &detail);
        let mut row = Element::row("status")
            .width(w - 2.0 * STATUS_X * k)
            .height(STATUS_H * k)
            .align_items(AlignItems::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
            .child(
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(hex(INK))
                    .margin_edges(0.0, 0.0, 0.0, 9.0 * k),
            )
            .child(
                Element::live_text(format!("detail-{state}"), &words)
                    .text_style(hint)
                    .color(hex(HINT))
                    .height(STATUS_H * k)
                    .margin_edges(0.0, 0.0, 0.0, 3.0 * k)
                    .shrink(1.0)
                    .min_width(0.0)
                    .motion(Prop::Opacity, Spring::critical_to_98(0.12))
                    .word_enter(Transition::fade())
                    .word_change(Transition::fade().opacity(0.2)),
            );
        if let Some(fraction) = self
            .kit_progress()
            .filter(|_| matches!(mode, Mode::Loading | Mode::Finishing))
        {
            row = row.child(Element::row("spacer").grow(1.0)).child(
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
                .height(2.0 * k)
                .width(150.0 * k),
            );
        }
        row
    }
}

#[cfg(test)]
mod r2_tests {
    use super::*;

    fn look(theme: OsdTheme, feature: &str) -> Look {
        let cfg = OsdConfig::default();
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut l = Look::new(&cfg, &plasma, String::new(), &fonts);
        l.theme = theme;
        l.features = [feature.to_string()].into();
        l
    }

    /// R2-24-a: the ink reveal writes with the stroke-write pen in River
    /// only, so a new word starts at 70 % ink or more and fills from there.
    #[test]
    fn the_ink_reveal_writes_with_the_pen() {
        assert!(look(OsdTheme::River, INK_REVEAL).on(glyph_paths::WRITE));
        assert!(!look(OsdTheme::Swiss, INK_REVEAL).on(glyph_paths::WRITE));
        assert_eq!(bleed_alpha(0.0), BLEED_ALPHA);
        assert_eq!(bleed_alpha(BLEED_SECS), 0.0);
        assert!((0..100).all(|i| bleed_alpha(i as f64 * 0.005) <= 0.15));
    }

    /// R2-24-b: the rim moves 3 px at most and the rings only come with
    /// the voice.
    #[test]
    fn the_panel_breathes_within_three_pixels() {
        for level in [0.0, 0.2, 0.5, 1.0, 3.0] {
            let i = rim_inset(level);
            assert!((0.0..=3.0).contains(&i), "{level} {i}");
        }
        assert!(ripples(3.0, 9.0, |_| 0.0).is_empty());
        assert!(ripples(3.0 + RIPPLE_LIFE, 3.0, |_| 0.5).is_empty());
        let rings = ripples(3.0, 9.0, |_| 0.5);
        assert!(!rings.is_empty());
        assert!(rings
            .iter()
            .all(|&(r, a)| r <= RIPPLE_REACH && a <= RIPPLE_ALPHA));
    }

    /// R2-24-c: the ribbon is a still line `FLAT_SECS` after the stop.
    #[test]
    fn the_ribbon_is_a_line_600_ms_after_the_stop() {
        let ribbon = Ribbon {
            levels: LevelTrack {
                levels: Arc::from(vec![0.8f32; 200]),
                newest_at: 4.0,
                rate: FRAME_HZ as f32,
                ..LevelTrack::default()
            },
            settle: Some(3.0),
            settle_secs: FLAT_SECS,
            ..Ribbon::default()
        };
        let rect = vo::scene::geom::Rect::new(0.0, 0.0, 400.0, 60.0);
        let swing = |t: f64| {
            (0..ribbon.strands)
                .flat_map(|i| ribbon.strand(rect, t, i).0)
                .map(|p| (p.y - 30.0).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(swing(3.0) > 3.0);
        assert!(swing(3.0 + FLAT_SECS as f64) < 1e-3);
    }
}
