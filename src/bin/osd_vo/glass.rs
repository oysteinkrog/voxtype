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
        let words = self.typo_words(words);
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
            // "Cancelled", "No speech" or "Failed".
            Mode::Cancelled => (
                status.state.clone(),
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
