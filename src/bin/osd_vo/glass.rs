//! The `glass` look of the vo popup, after concept 08, spatial glass: a
//! portrait card of frosted navy glass with a blue rim light. A mirrored
//! bar waveform glows across the top, the words sit centred in the middle
//! with the newest line at a fixed height (older lines rise above it), a
//! round microphone button in a glowing ring sits under them, and a centred
//! status row runs along the bottom.
//!
//! While the final text is made the ring turns into a spinner; when it
//! lands the button shows a check and the status reads "Done" with the word
//! count. A cancel greys the ring and dims the words. Under reduced motion
//! the glass draws no sheen, the bars and the ring hold still and the
//! check is drawn at once.

use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::Glass;
use vo::scene::path::{icons, make, Outline, StrokeStyle};

/// Colours sampled from the redesign.
const TINT: u32 = 0x0e1638;
const RIM: u32 = 0x7a9cff;
const ICE: u32 = 0xf3f6ff;
const INK: u32 = 0xf1f4fd;
const CYAN: u32 = 0x58c8ff;
const BLUE: u32 = 0x3d74ff;
const VIOLET: u32 = 0x8b6cff;
const DISC: u32 = 0x0c1846;
const HINT: u32 = 0xa9bff0;
const DIM: u32 = 0x6b7591;

/// The card in the redesign's units at `width_px = 600`; the look scales
/// with the popup's width. `ROOM` is kept round the card for its glow and
/// shadow.
const DESIGN_W: f32 = 600.0;
const CARD_W: f32 = 520.0;
const CARD_H: f32 = 600.0;
const ROOM: f32 = 16.0;
const RADIUS: f32 = 52.0;
/// The waveform's middle, half width and half height.
const WAVE_Y: f32 = 92.0;
const WAVE_HALF_W: f32 = 175.0;
const WAVE_HALF_H: f32 = 34.0;
const BAR_PITCH: f32 = 8.0;
const BAR_W: f32 = 3.2;
const BAR_SECS: f64 = 0.03;
/// The words: width, top of the text box, size and pitch. The newest line
/// is the box's last; older lines rise above it.
const TEXT_W: f32 = 448.0;
const TEXT_TOP: f32 = 166.0;
const TEXT_PX: f32 = 31.0;
const TEXT_LINE: f32 = 44.0;
pub(crate) const MAX_LINES: u32 = 4;
/// iv7 idle: the pace it draws at, the caustic's speed and its strongest
/// alpha, and the sheets' parallax drift and its period.
/// At 10 frames a second a spot moves 0.4 px a step.
const IDLE_FPS: f64 = 10.0;
const CAUSTIC_PX_S: f32 = 4.0;
const CAUSTIC_ALPHA: f32 = 0.08;
const DRIFT_PX: f32 = 3.0;
const DRIFT_SECS: f32 = 12.0;
/// How far the depth stack sits below the flat text: its sheets pad the
/// newest line 4 units down, so it lands where the flat line does.
const STACK_DROP: f32 = 4.0;
/// The microphone button's middle and its ring's radius.
const MIC_Y: f32 = 452.0;
const MIC_R: f32 = 42.0;
/// The status row's middle and height, and the progress line above it.
const STATUS_Y: f32 = 548.0;
const STATUS_H: f32 = 20.0;
const PROGRESS_W: f32 = 220.0;
const PROGRESS_GAP: f32 = 22.0;

// The words sit between the waveform and the button, and the button above
// the progress line and the status row, inside the card.
const _: () = assert!(TEXT_TOP > WAVE_Y + WAVE_HALF_H + 10.0);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < MIC_Y - MIC_R - 10.0);
const _: () = assert!(MIC_Y + MIC_R < STATUS_Y - PROGRESS_GAP - 4.0);
const _: () = assert!(STATUS_Y + STATUS_H / 2.0 < CARD_H - 20.0);

/// MO-84 in glass: a cool light, its accent the look's cyan, and softer
/// caustics and shadows than the default, which read as smoke on the navy.
pub(crate) fn letter_light() -> vo_3d::glyph_fx::Light {
    vo_3d::glyph_fx::Light {
        color: hex(ICE).fade(0.3),
        accent: hex(CYAN),
        shadow_len: 16.0,
        shadow: 0.15,
        caustics: 0.25,
        sheen: 0.35,
        edge_fade: 40.0,
        ..vo_3d::glyph_fx::Light::default()
    }
}

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn sans(look: &Look) -> u32 {
    look.fonts.system("Google Sans Flex:medium").unwrap_or(SANS)
}

fn ui(look: &Look) -> u32 {
    look.fonts.system("Google Sans Flex").unwrap_or(SANS)
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    left_style(look).centered()
}

/// The words' style before centring: an edited line is placed by its own
/// node, centred by the look.
fn left_style(look: &Look) -> TextStyle {
    let k = scale(look);
    TextStyle::new(sans(look), TEXT_PX * k, TEXT_LINE * k).tracking(-0.2 * k)
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

/// The width to give `Popup::depth_stack_lines` so its sheets come out
/// as wide as the text: the stack makes them `min(w * 0.84, w - 24k)`.
/// The lines then wrap at the text width as in the flat text, and the
/// newest line fits on its sheet.
fn stack_span(look: &Look) -> f32 {
    let (tw, k) = (text_width(look), scale(look));
    (tw / 0.84).max(tw + 24.0 * k)
}

pub(crate) fn surface(look: &Look) -> Size {
    let k = scale(look);
    Size::new(
        ((CARD_W + 2.0 * ROOM) * k).ceil(),
        ((CARD_H + 2.0 * ROOM) * k).ceil(),
    )
}

/// The card's top in the surface: below the room for its glow.
pub(crate) fn panel_top(look: &Look) -> f32 {
    ROOM * scale(look)
}

/// The middle of the newest line below the surface's top. The card keeps
/// its size and the newest line its place, whatever the line count.
pub(crate) fn newest_line_centre(look: &Look) -> f32 {
    (ROOM + TEXT_TOP + (MAX_LINES as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// The mirrored bar waveform: bright blue in the middle, violet towards
/// the ends, where it thins to dots.
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
            _ => self.mode_at + 0.6,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let n = (WAVE_HALF_W / BAR_PITCH).floor() as i32;
        let dim = match self.mode {
            Mode::Finishing | Mode::Done => 1.0 - 0.55 * ease_out((t - self.mode_at) / 0.4),
            Mode::Cancelled => 0.3,
            _ => 1.0,
        };
        let t_eff = if self.still {
            self.shown_at
        } else {
            self.finish_at.map_or(t, |f| t.min(f))
        };
        let sweep = (((t - self.shown_at) / 1.4).fract()) as f32 * 2.0 - 1.0;
        for i in -n..=n {
            let reach = i.unsigned_abs() as f32 / n as f32;
            let x = cx + i as f32 * BAR_PITCH * k;
            let c = if reach < 0.5 {
                mix(hex(CYAN), hex(BLUE), reach * 2.0)
            } else {
                mix(hex(BLUE), hex(VIOLET), (reach - 0.5) * 2.0)
            }
            .fade(dim);
            if self.mode == Mode::Loading {
                // A soft light runs along the dots while the model loads.
                let near = if self.still {
                    0.0
                } else {
                    (1.0 - ((i as f32 / n as f32) - sweep).abs() * 5.0).clamp(0.0, 1.0)
                };
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    (1.2 + near) * k,
                    Paint::solid(c.fade(0.35 + 0.65 * near)),
                ));
                continue;
            }
            let at = t_eff - i.unsigned_abs() as f64 * BAR_SECS;
            let level = if matches!(self.mode, Mode::Cancelled) {
                0.0
            } else if self.still {
                // Reduced motion: a fixed mid-height shape, not the voice.
                0.3
            } else {
                self.levels.at(at)
            };
            let taper = (1.0 - reach.powf(1.6)).max(0.0);
            let half = (level * 1.9).min(1.0) * taper * WAVE_HALF_H * k;
            if half < 2.2 * k {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    1.3 * k,
                    Paint::solid(c.fade(0.8)),
                ));
                continue;
            }
            let bar = Rect::new(x - BAR_W * k / 2.0, cy - half, BAR_W * k, 2.0 * half);
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W * k / 2.0),
                Paint::glow(c.fade(0.5), 7.0 * k, 0.6),
            ));
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W * k / 2.0),
                Paint::linear(
                    Point::new(x, cy - half),
                    mix(c, hex(ICE), 0.35),
                    Point::new(x, cy),
                    c,
                ),
            ));
        }
    }
}

/// The round microphone button in its glowing ring; a spinner while the
/// final text is made, a check once it lands, a grey cross on a cancel.
/// iv7 idle (RD-02): a faint caustic, light bent through the glass, on
/// two layers that drift past each other above the newest line. Clear of
/// the newest line, at a capped rate.
#[derive(Clone)]
struct Caustic {
    shown_at: f64,
    /// The band it may light, card y, top and bottom.
    band: (f32, f32),
    w: f32,
    k: f32,
}

impl Caustic {
    /// Spots per layer.
    const SPOTS: u32 = 5;

    fn draw(&self, t: f64, p: &mut Painter) {
        let k = self.k;
        let s = ((t * IDLE_FPS).floor() / IDLE_FPS - self.shown_at) as f32;
        let (top, bottom) = self.band;
        for layer in 0..2u32 {
            // The far layer moves at half the near one's pace and drifts
            // the other way: the parallax between the two sheets.
            let pace = if layer == 0 { 1.0 } else { 0.5 };
            let sway = if layer == 0 { 1.0 } else { -1.0 };
            let lift =
                sway * DRIFT_PX * k * 0.5 * (1.0 - (s / DRIFT_SECS * std::f32::consts::TAU).cos());
            let r = (if layer == 0 { 34.0 } else { 48.0 }) * k;
            for i in 0..Self::SPOTS {
                let seed = layer * 31 + i * 7 + 3;
                // Inside the card: a spot runs from one side to the other
                // and fades out at each end before it starts over.
                let span = (self.w - 2.0 * r).max(1.0);
                let x = r + (hash(seed) * span + CAUSTIC_PX_S * k * pace * s) % span;
                let ends = ((x - r).min(self.w - r - x) / (24.0 * k)).clamp(0.0, 1.0);
                let room = (bottom - top - 2.0 * r).max(0.0);
                let y = top + r + hash(seed ^ 0x5bd1) * room - lift.abs();
                if y + r > bottom || y - r < top - 2.0 * r {
                    continue;
                }
                let a = CAUSTIC_ALPHA * (0.6 + 0.4 * hash(seed ^ 0x9e37)) * ends;
                p.shape(ShapeInstance::circle(
                    Point::new(x, y),
                    r * 0.35,
                    Paint::glow(
                        mix(hex(CYAN), hex(VIOLET), hash(seed ^ 0x1f)).fade(a),
                        r * 0.65,
                        0.5,
                    ),
                ));
            }
        }
    }
}

/// A repeatable 0..1 for `seed`.
fn hash(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

#[derive(Clone)]
struct Mic {
    mode: Mode,
    mode_at: f64,
    levels: Levels,
    k: f32,
    still: bool,
}

impl Mic {
    fn until(&self) -> f64 {
        match self.mode {
            Mode::Listening | Mode::Finishing if !self.still => f64::INFINITY,
            _ => self.mode_at + 0.5,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rad = MIC_R * k;
        let grow = if self.still {
            1.0
        } else {
            ease_out((t - self.mode_at) / 0.35)
        };
        let lit: f32 = match self.mode {
            Mode::Cancelled => 0.0,
            Mode::Loading => 0.5,
            _ => 1.0,
        };
        // The voice lifts the ring's glow a little while listening.
        let voice = if self.mode == Mode::Listening && !self.still {
            (self.levels.at(t) * 1.6).min(1.0)
        } else {
            0.0
        };
        let (c0, c1) = if lit > 0.0 {
            (hex(CYAN), hex(VIOLET))
        } else {
            (hex(DIM), hex(DIM))
        };
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::glow(
                hex(BLUE).fade((0.35 + 0.3 * voice) * lit.max(0.25)),
                16.0 * k,
                0.7,
            ),
        ));
        p.shape(ShapeInstance::circle(
            c,
            rad,
            Paint::linear(
                Point::new(c.x, c.y - rad),
                mix(hex(DISC), hex(BLUE), 0.25),
                Point::new(c.x, c.y + rad),
                hex(DISC),
            ),
        ));
        let ring = StrokeStyle::solid(3.0 * k, c0)
            .linear(
                Point::new(c.x - rad, c.y - rad),
                c0,
                Point::new(c.x + rad, c.y + rad),
                c1,
            )
            .glow(6.0 * k);
        match self.mode {
            Mode::Finishing => {
                p.stroke(
                    &make::ring(c, rad),
                    &StrokeStyle::solid(3.0 * k, hex(BLUE).fade(0.25)),
                );
                let spin = if self.still { 0.0 } else { t - self.mode_at };
                p.stroke(&make::spinner(c, rad, spin), &ring);
            }
            _ => p.stroke(&make::ring(c, rad), &ring),
        }
        // An inner hairline, as the glass button's lip.
        p.stroke(
            &make::ring(c, rad * 0.8),
            &StrokeStyle::solid(1.0 * k, hex(RIM).fade(0.35 * lit.max(0.4))),
        );
        let icon = |outline: Outline, size: f32, colour: Color, trim: f32| {
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
        let (o, style) = match self.mode {
            Mode::Done => icon(
                Outline::new(&icons::check(), 0.05),
                rad * 0.95,
                hex(ICE),
                grow,
            ),
            Mode::Cancelled => icon(Outline::new(&icons::x(), 0.05), rad * 0.8, hex(DIM), grow),
            _ => icon(
                Outline::new(&icons::mic(), 0.05),
                rad * 0.95,
                mix(hex(DIM), hex(ICE), lit.max(0.5)),
                1.0,
            ),
        };
        p.outline(&o, &style);
    }
}

impl Popup {
    pub(crate) fn glass_view(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let s = surface(l);
        let root = Element::column("root").size(s.w, s.h);
        if self.closing {
            return root;
        }
        root.child(at(self.glass_card(), ROOM * k, ROOM * k))
    }

    /// The glass card with everything on it.
    fn glass_card(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let still = self.still();
        let (w, h) = (CARD_W * k, CARD_H * k);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let glass = Glass {
            accent: hex(CYAN),
            rim: 1.2,
            bevel: 16.0 * k,
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
        let mic = Mic {
            mode,
            mode_at,
            levels: self.kit_raw_levels(),
            k,
            still,
        };
        let wave_box = Size::new(
            2.0 * (WAVE_HALF_W + 16.0) * k,
            2.0 * (WAVE_HALF_H + 14.0) * k,
        );
        let mic_box = 2.0 * (MIC_R + 22.0) * k;
        let until = wave.until();
        let mic_until = mic.until();
        Element::panel("card")
            .size(w, h)
            .radius(RADIUS * k)
            .glass(glass)
            .border(Color::hex(RIM, 0.5), 1.2 * k)
            .glow(hex(BLUE).fade(0.3), 10.0 * k, 0.6)
            .rim(Color::hex(RIM, 0.75), 1.6 * k, 0.2, -0.375, 0.0)
            .presence(Transition::fade().offset(0.0, 8.0).scale(0.98), exit)
            .child(at(
                Element::canvas("wave", until, move |t, r, p| wave.draw(t, r, p))
                    .size(wave_box.w, wave_box.h),
                (w - wave_box.w) / 2.0,
                WAVE_Y * k - wave_box.h / 2.0,
            ))
            .children(self.limit_line(
                Point::new((w - text_width(l)) / 2.0, TEXT_TOP * k),
                Size::new(w, h),
                hex(INK).fade(0.55),
                hex(CYAN),
            ))
            .children(self.glass_caustic(mode, k, w))
            .child(self.glass_text(mode, k, w))
            .child(at(
                Element::canvas("mic", mic_until, move |t, r, p| mic.draw(t, r, p))
                    .size(mic_box, mic_box),
                (w - mic_box) / 2.0,
                MIC_Y * k - mic_box / 2.0,
            ))
            .children(self.glass_progress(mode, k, w))
            .child(at(
                self.glass_status(mode, mode_at, k, w),
                0.0,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The words, centred, the newest line at the box's foot.
    fn glass_text(&self, mode: Mode, k: f32, w: f32) -> Element<Msg> {
        let l = &self.look;
        let tw = text_width(l);
        let lines = self.lines.clamp(1, MAX_LINES);
        let x = (w - tw) / 2.0;
        let y = (TEXT_TOP + (MAX_LINES - lines) as f32 * TEXT_LINE) * k;
        if self.edit_view.is_some() {
            return at(self.glass_edit_words(k), x, y).motion(Prop::Position, Motion::Snap);
        }
        if self.glass_stacking(mode) {
            return at(self.glass_stack(k), x, y + STACK_DROP * k)
                .motion(Prop::Position, Motion::Snap);
        }
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
            .word_enter(self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0)));
        let words = self.letter_fx_words(self.typo_words(words));
        // The magnetic caret stands at the end of the text only while it
        // is spoken.
        let words = if mode == Mode::Listening {
            words.wild_motion(self)
        } else {
            words
        };
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
        let text = if self.shown.is_empty() {
            // No words yet: the forming cue grows from the middle of the
            // newest line, where centred words start.
            let tail = model_signals::TailText {
                style: left_style(l),
                width: tw / 2.0,
                rows: 1,
            };
            self.signal_marks_in(text, &[], &tail, (tw / 2.0, 0.0), hex(INK))
        } else {
            self.signal_marks(text, &self.shown, tw, (0.0, 0.0), hex(INK))
        };
        // The column steps up a line at once as the text wraps, so the
        // newest line never leaves its height; a spring would carry it.
        at(
            self.with_hearing(text, &l.text_style(), tw, hex(INK), hex(CYAN)),
            x,
            y,
        )
        .motion(Prop::Position, Motion::Snap)
    }

    /// The idle caustic while the popup listens, above the newest line;
    /// none under reduced motion, while editing or once the words end.
    fn glass_caustic(&self, mode: Mode, k: f32, w: f32) -> Option<Element<Msg>> {
        if mode != Mode::Listening || self.still() || self.edit_view.is_some() {
            return None;
        }
        let newest_top = (TEXT_TOP + (MAX_LINES - 1) as f32 * TEXT_LINE) * k;
        let caustic = Caustic {
            shown_at: self.shown_at,
            band: ((WAVE_Y + WAVE_HALF_H) * k, newest_top - 4.0 * k),
            w,
            k,
        };
        Some(at(
            Element::canvas("caustic", f64::INFINITY, move |t, r, p| {
                p.layer(Affine::translate(r.x, r.y), 1.0, |p| caustic.draw(t, p))
            })
            .size(w, newest_top)
            .presence(Transition::fade(), Transition::fade()),
            0.0,
            0.0,
        ))
    }

    /// MO-86: the depth stack draws the words while they are spoken and
    /// made final; the hand-off, a cancel and an edit take the flat text.
    fn glass_stacking(&self, mode: Mode) -> bool {
        self.look.on(wild_space::DEPTH_STACK)
            && matches!(mode, Mode::Listening | Mode::Finishing)
            && self.delivered().is_none()
            && !self.shown.is_empty()
    }

    /// The words on the depth stack: the newest line flat on the glass,
    /// the older ones receding on sheets above it.
    fn glass_stack(&self, k: f32) -> Element<Msg> {
        let l = &self.look;
        let (style, tw) = (left_style(l), text_width(l));
        let span = stack_span(l);
        let rows = self.lines.clamp(1, MAX_LINES);
        let lines = self.text_lines(&style, tw);
        let skip = lines.len().saturating_sub(rows as usize);
        let stack = self.depth_stack_lines(
            &lines[skip..],
            span,
            rows,
            style,
            k,
            hex(INK),
            hex(BLUE),
            // A word lands at full ink: its sheet already rises from 70 %
            // ink, and the magnet's own fade on top breaks the reading
            // line's ink floor.
            &|words| {
                self.letter_fx_words(self.typo_words(words))
                    .wild_motion(self)
                    .word_enter(Transition::NONE)
            },
            &|line, words, room| {
                let tail = model_signals::TailText {
                    style,
                    width: room,
                    rows: 1,
                };
                self.signal_marks_in(line, words, &tail, (0.0, 0.0), hex(INK))
            },
            self.glass.hold.as_ref(),
        );
        // The stack is wider than the text by its sheets' margin; centre
        // it on the text so the sheets line up with the text box.
        let text = Element::column("text")
            .width(tw)
            .height(rows as f32 * TEXT_LINE * k)
            .child(at(stack, (tw - span) / 2.0, 0.0));
        self.letter_fx_layers(text, tw)
    }

    /// While editing: one live text per line, each centred by its own
    /// width, so the edit caret lands on its glyph (`GlassSite`).
    fn glass_edit_words(&self, k: f32) -> Element<Msg> {
        let l = &self.look;
        let (style, w) = (left_style(l), text_width(l));
        let exit = if self.edit_tapped() {
            Transition::NONE
        } else {
            Transition::fade()
        };
        let mut text = Element::column("text").size(w, self.lines as f32 * TEXT_LINE * k);
        for (i, (words, lw)) in self.text_lines(&style, w).iter().enumerate() {
            let key = words
                .first()
                .map_or_else(|| format!("slot-{i}"), |w| w.id.to_string());
            let line = Element::live_text(format!("line-{key}"), words)
                .text_style(style)
                .color(hex(INK))
                .width(w)
                .word_enter(Transition::NONE)
                .presence(Transition::NONE, exit);
            text = text.child(at(
                line,
                ((w - lw) / 2.0).max(0.0),
                i as f32 * TEXT_LINE * k,
            ));
        }
        text
    }

    /// A thin line over the status while a model loads or the final text
    /// is made.
    fn glass_progress(&self, mode: Mode, k: f32, w: f32) -> Option<Element<Msg>> {
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
            (w - PROGRESS_W * k) / 2.0,
            (STATUS_Y - PROGRESS_GAP) * k,
        ))
    }

    /// The status row, centred: a dot, the state word and the detail; on
    /// done a check, "Done" and the word count in cyan.
    fn glass_status(&self, mode: Mode, mode_at: f64, k: f32, w: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(ui(l), 15.0 * k, STATUS_H * k);
        let hint = TextStyle::new(ui(l), 14.0 * k, STATUS_H * k);
        let status = self.status();
        let done = mode == Mode::Done;
        let (state, detail, dot) = match mode {
            Mode::Done => {
                let n = self.shown.len();
                let count = format!("  ·  {n} word{}", if n == 1 { "" } else { "s" });
                (self.delivery_word().to_string(), count, hex(CYAN))
            }
            // "Cancelled", "No speech" or "Failed": how it ended, even when
            // the daemon still reports a delivery.
            Mode::Cancelled => (
                self.ending().map_or("Cancelled", Ending::label).to_string(),
                "  ·  nothing was typed".to_string(),
                hex(DIM),
            ),
            _ => (status.state.clone(), status.detail.clone(), hex(BLUE)),
        };
        let still = self.still();
        let mark = Element::canvas("dot", mode_at + 0.4, move |t, r, p| {
            let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            if done {
                // A small check in place of the dot.
                let k2 = if still {
                    1.0
                } else {
                    ease_out((t - mode_at) / 0.3)
                };
                let unit = 13.0 * k / 24.0;
                let o = Outline::new(&icons::check(), 0.05).transform(
                    &Affine::translate(-12.0, -12.0)
                        .then(&Affine::scale(unit))
                        .then(&Affine::translate(c.x, c.y)),
                );
                let len = o.length();
                p.outline(
                    &o.trim(0.0, len * k2),
                    &StrokeStyle::solid(2.4 * unit, hex(CYAN)),
                );
                return;
            }
            p.shape(ShapeInstance::circle(
                c,
                4.0 * k,
                Paint::glow(dot.fade(0.5), 5.0 * k, 0.5),
            ));
            p.shape(ShapeInstance::circle(c, 4.0 * k, Paint::solid(dot)));
        })
        .size(14.0 * k, 14.0 * k);
        let words = super::kit::detail_words(&state, &detail);
        let (state_c, detail_c) = if done {
            (hex(CYAN), hex(CYAN))
        } else {
            (hex(INK), hex(HINT))
        };
        let row = Element::row("status")
            .height(STATUS_H * k)
            .align_items(AlignItems::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
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
            );
        Element::row("status-row")
            .width(w)
            .height(STATUS_H * k)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .child(row)
    }
}

/// What only the glass look keeps: the depth stack's newest line and the
/// line held flat as it leaves the reading place (MO-86).
#[derive(Default)]
pub(crate) struct State {
    newest: Option<(vo_core::words::WordId, u32)>,
    hold: Option<vo::scene::depth_stack::Hold>,
    wake: Option<f64>,
}

impl Popup {
    /// Notes when the newest line of the depth stack changes, and holds
    /// the line that leaves the reading place flat a line pitch above the
    /// new one before it moves back, as jarvis does.
    pub(crate) fn sync_glass(&mut self, t: f64) {
        if self.look.theme != OsdTheme::Glass || !self.look.on(wild_space::DEPTH_STACK) {
            return;
        }
        let style = left_style(&self.look);
        let lines = self.stable_lines(&style, text_width(&self.look));
        let first = |l: &(Vec<Word>, f32)| l.0.first().map(|w| w.id);
        let newest = lines.last().and_then(first).map(|id| (id, self.lines));
        if let (Some((was, _)), Some((id, _))) = (self.glass.newest, newest) {
            let behind = lines.len().checked_sub(2).and_then(|i| first(&lines[i]));
            if id != was && behind == Some(was) {
                // The newest line keeps its height as the box grows
                // upward, so the line that leaves it always glides up a
                // row before it turns back, as in a full jarvis box.
                let hold = vo::scene::depth_stack::Hold {
                    key: wild_space::depth_stack_key(was),
                    since: t,
                    secs: wild_space::GLIDE_SECS,
                    rise: style.line_height,
                    glide: true,
                };
                // The stack's spring is the default one (`wild_space`).
                self.glass.wake =
                    Some(vo::scene::depth_stack::DepthStack::default().newest_shows_at(&hold));
                self.glass.hold = Some(hold);
            }
        }
        if newest.is_some() {
            self.glass.newest = newest;
        }
    }

    /// Builds the view again when the new stack line shows.
    pub(crate) fn arm_glass_wake(&mut self, cx: &mut Cx<Msg>) {
        if let Some(at) = self.glass.wake.take() {
            cx.after((at - cx.t()).max(0.0) + 0.01, Msg::Loop);
        }
    }
}

/// Glass's edit adapter: the words are centred, so each edited line is
/// placed by its own node, as particles does.
pub(crate) struct GlassSite;

impl edit_site::EditSite for GlassSite {
    fn text_box(&self, look: &Look) -> edit_site::TextBox {
        edit_site::TextBox {
            style: left_style(look),
            width: text_width(look),
        }
    }

    fn place(&self, look: &Look, el: Element<Msg>, view: &edit::EditView) -> Element<Msg> {
        edit_site::place_lines(self, look, el, view)
    }
}
