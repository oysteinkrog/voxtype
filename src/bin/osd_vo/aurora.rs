//! The `aurora` look of the vo popup, after concept 19, the aurora mesh: a
//! rounded square card filled with a soft mesh gradient, warm orange and
//! magenta on the left, deep indigo in the middle and blue to cyan on the
//! right, inside a rim that runs the same colours round the card. The
//! microphone sits in a glowing ring at the top, a thin line of coloured
//! light under it rises in humps with the voice, the words are centred in
//! white under the line and grow downward, and a centred status row runs
//! along the bottom.
//!
//! The mesh is a GPU material (`Material::aurora`); the CPU first frame
//! draws its plain gradient. It brightens and its colours swell with the
//! voice. While the final text is made the line lowers; when the text lands
//! the line flattens and the status turns green. A cancel greys the mesh
//! and the rim and dims the words. Under reduced motion the mesh holds one
//! still state.

use super::glyph_paths::PathsText;
use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, Rect};
use vo::scene::material::Material;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};

/// Colours sampled from the redesign: the mesh's four blobs (bottom
/// middle, upper right, middle, lower left), the line's stops from left to
/// right, and the type.
const MESH_LOW: u32 = 0x8a2cdc;
const MESH_RIGHT: u32 = 0x2a6cf2;
const MESH_MID: u32 = 0x181256;
const MESH_LEFT: u32 = 0xf24a3a;
/// The CPU first frame's plain gradient, top and bottom.
const PLAIN_TOP: u32 = 0x3a2a9a;
const PLAIN_BOTTOM: u32 = 0x5a2aa6;
const ORANGE: u32 = 0xff7a45;
const PINK: u32 = 0xff4fa8;
const VIOLET: u32 = 0xa35bff;
const BLUE: u32 = 0x4f7dff;
const CYAN: u32 = 0x3fd4ff;
const INK: u32 = 0xfbf9ff;
const HINT: u32 = 0xe2dcff;
const DIM: u32 = 0x7d7f99;
const GREY: u32 = 0x3a3d52;
const ORB: u32 = 0x2a1a62;
const RED: u32 = 0xff5a5f;
const GREEN: u32 = 0x3ddc97;

/// The card in the redesign's units at `width_px = 600`; the look scales
/// with the popup's width. `ROOM` is kept round the card for its glow.
const DESIGN_W: f32 = 600.0;
const CARD_W: f32 = 560.0;
const CARD_H: f32 = 460.0;
const ROOM: f32 = 16.0;
const RADIUS: f32 = 38.0;
/// The microphone's ring.
const MIC_Y: f32 = 76.0;
const MIC_R: f32 = 34.0;
/// The line of light under the microphone: its middle, ends and how high
/// its humps rise.
const WAVE_Y: f32 = 150.0;
const WAVE_X0: f32 = 36.0;
const WAVE_X1: f32 = 524.0;
const WAVE_HALF: f32 = 26.0;
const WAVE_POINTS: usize = 121;
/// Seconds of audio from the line's middle out to each end.
const WAVE_SECS: f64 = 0.6;
/// Levels under this are room tone and draw flat.
const WAVE_GATE: f32 = 0.04;
/// The words: centred in this width from this top, growing downward. The
/// clip starts under the line of light.
const TEXT_W: f32 = 492.0;
const TEXT_PX: f32 = 33.0;
const TEXT_LINE: f32 = 46.0;
const TEXT_TOP: f32 = 192.0;
const CLIP_TOP: f32 = 178.0;
const MAX_LINES: u32 = 4;
/// The status row's middle and height.
const STATUS_Y: f32 = 424.0;
const STATUS_H: f32 = 20.0;
const STATUS_PX: f32 = 14.0;
/// The mesh's energy at rest, and how much the voice adds.
const CALM: f32 = 0.25;
const SWELL: f32 = 0.6;
const SETTLE_SECS: f64 = 0.35;

// The line sits under the microphone, the words between the line and the
// status row, and the status row inside the card.
const _: () = assert!(MIC_Y + MIC_R < WAVE_Y - WAVE_HALF);
const _: () = assert!(CLIP_TOP > WAVE_Y + 8.0 && CLIP_TOP < TEXT_TOP);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < STATUS_Y - STATUS_H);
const _: () = assert!(STATUS_Y + STATUS_H / 2.0 < CARD_H - 20.0);
const _: () = assert!(TEXT_W < CARD_W - 2.0 * 20.0);

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

fn face(look: &Look) -> u32 {
    look.fonts
        .system("Google Sans Flex")
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
        .tracking(-0.3 * k)
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
    Size::new(
        ((CARD_W + 2.0 * ROOM) * k).ceil(),
        ((CARD_H + 2.0 * ROOM) * k).ceil(),
    )
}

/// The card's top in the surface: below the room for its glow.
pub(crate) fn panel_top(look: &Look) -> f32 {
    ROOM * scale(look)
}

/// The middle of the newest line below the surface's top. The words grow
/// downward from the top of the text box.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (ROOM + TEXT_TOP + (lines.clamp(1, MAX_LINES) as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// How far a change of state has gone, 0 to 1.
fn change(t: f64, at: f64, still: bool) -> f32 {
    if still {
        1.0
    } else {
        ease_out((t - at) / SETTLE_SECS)
    }
}

/// The rim round the card and the microphone in its ring, in the card's
/// space grown by `ROOM` on every side.
#[derive(Clone, Copy)]
struct Frame {
    k: f32,
    mode: Mode,
    mode_at: f64,
    still: bool,
}

impl Frame {
    fn until(&self) -> f64 {
        self.mode_at + SETTLE_SECS + 0.1
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let c = change(t, self.mode_at, self.still);
        let (grey, lit) = match self.mode {
            Mode::Cancelled => (c, 1.0 - c),
            Mode::Loading => (0.0, 0.5),
            _ => (0.0, 1.0),
        };
        let tint = |h: u32| mix(hex(h), hex(DIM), grey);
        let room = ROOM * k;
        let (x, y) = (r.x + room, r.y + room);
        let (w, h) = (r.w - 2.0 * room, r.h - 2.0 * room);
        // The rim: warm on the left, cool on the right, with a soft glow
        // outside and a thin bright edge.
        let middle = Point::new(x + w / 2.0, y + h / 2.0);
        let rim = make::panel(x, y, w, h, RADIUS * k);
        p.stroke(
            &rim,
            &StrokeStyle::solid(3.0 * k, tint(PINK).fade(0.35))
                .conic(
                    middle,
                    std::f32::consts::PI,
                    tint(ORANGE).fade(0.4),
                    tint(CYAN).fade(0.4),
                )
                .glow(12.0 * k),
        );
        p.stroke(
            &rim,
            &StrokeStyle::solid(1.4 * k, tint(PINK)).conic(
                middle,
                std::f32::consts::PI,
                tint(PINK).fade(0.9),
                tint(CYAN).fade(0.9),
            ),
        );
        // The microphone in its ring.
        let o = Point::new(x + w / 2.0, y + MIC_Y * k);
        let rad = MIC_R * k;
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::glow(tint(VIOLET).fade(0.5 * lit), 14.0 * k, 0.5),
        ));
        p.shape(ShapeInstance::circle(
            o,
            rad,
            Paint::solid(mix(hex(ORB), hex(GREY), grey).fade(0.55)),
        ));
        p.stroke(
            &make::ring(o, rad),
            &StrokeStyle::solid(2.0 * k, tint(PINK))
                .conic(o, std::f32::consts::PI, tint(PINK), tint(BLUE))
                .glow(5.0 * k * lit),
        );
        let unit = rad / 24.0 * 0.85;
        let mic = Outline::new(&icons::mic(), 0.05).transform(
            &Affine::translate(-12.0, -12.0)
                .then(&Affine::scale(unit))
                .then(&Affine::translate(o.x, o.y)),
        );
        p.outline(
            &mic,
            &StrokeStyle::solid(1.9 * unit, mix(hex(DIM), hex(INK), 0.4 + 0.6 * lit)),
        );
    }
}

/// The line of light under the microphone: humps in the middle as high as
/// the voice, the newest audio in the middle and older audio out to the
/// ends, orange through pink, violet and blue to cyan.
#[derive(Clone)]
struct Wave {
    k: f32,
    levels: Levels,
    mode: Mode,
    mode_at: f64,
    finish_at: Option<f64>,
    still: bool,
}

/// The line's colour stops from left to right; the ends fade out.
const STOPS: [(f32, u32, f32); 7] = [
    (0.0, ORANGE, 0.0),
    (0.14, ORANGE, 1.0),
    (0.32, PINK, 1.0),
    (0.5, VIOLET, 1.0),
    (0.68, BLUE, 1.0),
    (0.86, CYAN, 1.0),
    (1.0, CYAN, 0.0),
];

impl Wave {
    fn until(&self, now: f64) -> f64 {
        match self.mode {
            Mode::Loading | Mode::Listening => f64::INFINITY,
            _ => self.mode_at.max(now) + SETTLE_SECS + 0.1,
        }
    }

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let cy = r.y + r.h * 0.6;
        let t_eff = self.finish_at.map_or(t, |f| t.min(f));
        let c = change(t, self.mode_at, self.still);
        let (height, grey) = match self.mode {
            Mode::Loading => (0.0, 0.4),
            Mode::Finishing => (1.0 - 0.6 * c, 0.0),
            Mode::Done => (0.4 * (1.0 - c), 0.0),
            Mode::Cancelled => (1.0 - c, c),
            Mode::Listening => (1.0, 0.0),
        };
        let pts: Vec<Point> = (0..WAVE_POINTS)
            .map(|i| {
                let u = i as f32 / (WAVE_POINTS - 1) as f32;
                let d = (2.0 * u - 1.0).abs();
                let raw = self.levels.at(t_eff - d as f64 * WAVE_SECS);
                // Room tone stays flat, so a silence holds still.
                let raw = if raw < WAVE_GATE { 0.0 } else { raw };
                let bell = (-(d / 0.5).powi(2)).exp();
                let hump = 0.5 + 0.5 * (std::f32::consts::TAU * 4.2 * u + 1.3).sin();
                let level = (raw * 2.2).min(1.0) * bell * (0.3 + 0.7 * hump) * height;
                Point::new(r.x + u * r.w, cy - level * WAVE_HALF * k)
            })
            .collect();
        let colour = |h: u32, a: f32| mix(hex(h), hex(DIM), grey).fade(a);
        // One glow under the whole line, short of its faded ends: a glow
        // per colour stop drew a bright bead at each join.
        let n = WAVE_POINTS - 1;
        let run = &pts[n / 10..=n - n / 10];
        let (p0, p1) = (run[0], run[run.len() - 1]);
        p.stroke(
            &Contour::polyline(run, false),
            &StrokeStyle::solid(2.0 * k, colour(VIOLET, 0.6))
                .linear(p0, colour(PINK, 0.6), p1, colour(BLUE, 0.6))
                .glow(8.0 * k),
        );
        for pair in STOPS.windows(2) {
            let (u0, h0, a0) = pair[0];
            let (u1, h1, a1) = pair[1];
            let (i0, i1) = (
                (u0 * n as f32).round() as usize,
                (u1 * n as f32).round() as usize,
            );
            let run = &pts[i0..=i1];
            let (p0, p1) = (run[0], run[run.len() - 1]);
            let line = Contour::polyline(run, false);
            p.stroke(
                &line,
                &StrokeStyle::solid(2.0 * k, colour(h0, a0)).linear(
                    p0,
                    colour(h0, a0),
                    p1,
                    colour(h1, a1),
                ),
            );
        }
    }
}

/// The look's transcript for the outline features.
fn aurora_paths(look: &Look) -> PathsText {
    PathsText {
        style: look.text_style(),
        width: look.text_width(),
        line: TEXT_LINE * scale(look),
        ink: hex(INK),
        nib: hex(CYAN),
    }
}

impl Popup {
    pub(crate) fn aurora_view(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let s = surface(l);
        let root = Element::column("root").size(s.w, s.h);
        if self.closing {
            return root;
        }
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        root.child(at(self.aurora_card(mode, mode_at), ROOM * k, ROOM * k))
    }

    /// The mesh's energy now: calm at rest, swelling with the voice. Fixed
    /// under reduced motion and once the dictation stops.
    fn aurora_energy(&self, mode: Mode) -> f32 {
        if self.still() || mode != Mode::Listening {
            return CALM;
        }
        let levels = self.kit_levels();
        let n = levels.levels.len();
        let recent = &levels.levels[n.saturating_sub(6)..];
        let voice = recent.iter().sum::<f32>() / recent.len().max(1) as f32;
        let voice = if voice < WAVE_GATE {
            0.0
        } else {
            (voice * 2.2).min(1.0)
        };
        // In steps, so small changes in the level do not redraw the card.
        CALM + SWELL * (voice * 20.0).round() / 20.0
    }

    /// The card: the mesh, the rim and microphone, the line of light, the
    /// words and the status row.
    fn aurora_card(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let still = self.still();
        let (w, h) = (CARD_W * k, CARD_H * k);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let (mesh, energy) = match mode {
            Mode::Cancelled => (
                [MESH_LOW, MESH_RIGHT, MESH_MID, MESH_LEFT].map(|c| mix(hex(c), hex(GREY), 0.7)),
                0.0,
            ),
            Mode::Loading => (
                [MESH_LOW, MESH_RIGHT, MESH_MID, MESH_LEFT]
                    .map(|c| mix(hex(c), hex(MESH_MID), 0.35)),
                0.0,
            ),
            _ => (
                [MESH_LOW, MESH_RIGHT, MESH_MID, MESH_LEFT].map(hex),
                self.aurora_energy(mode),
            ),
        };
        let material = Material::aurora(&mesh).energy(energy).size(0.36).speed(0.0);
        let frame = Frame {
            k,
            mode,
            mode_at,
            still,
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
        let wave_h = 2.0 * (WAVE_HALF + 12.0) * k;
        let lines = self.lines.clamp(1, MAX_LINES);
        let room = ROOM * k;
        Element::layer("card")
            .size(w, h)
            .presence(Transition::fade().offset(0.0, 8.0).scale(0.98), exit)
            .child(
                Element::panel("mesh")
                    .size(w, h)
                    .radius(RADIUS * k)
                    .fill_gradient(hex(PLAIN_TOP), hex(PLAIN_BOTTOM))
                    .material(material),
            )
            .child(at(
                Element::canvas("frame", frame.until(), move |t, r, p| frame.draw(t, r, p))
                    .size(w + 2.0 * room, h + 2.0 * room),
                -room,
                -room,
            ))
            .child(at(
                Element::canvas("wave", wave_until, move |t, r, p| wave.draw(t, r, p))
                    .size((WAVE_X1 - WAVE_X0) * k, wave_h),
                WAVE_X0 * k,
                WAVE_Y * k - wave_h * 0.6,
            ))
            .children(self.limit_line(
                Point::new((CARD_W - TEXT_W) / 2.0 * k, TEXT_TOP * k),
                Size::new(w, h),
                hex(INK).fade(0.55),
                hex(CYAN),
            ))
            .child(self.aurora_text_clip(mode, lines, k))
            .child(at(
                self.aurora_status(mode, mode_at, k, w),
                0.0,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The words, clipped under the line of light, so a line that leaves on
    /// an overflow fades out before it reaches the line.
    fn aurora_text_clip(&self, mode: Mode, lines: u32, k: f32) -> Element<Msg> {
        // Across the whole card: a centred line that grows slides to its
        // new middle, and a new word rides along past the text box until
        // it gets there.
        let m = 10.0 * k;
        let y = (TEXT_TOP - CLIP_TOP) * k;
        let h = y + lines as f32 * TEXT_LINE * k + m;
        let clip = Element::layer("text-clip")
            .size(CARD_W * k, h)
            .clip()
            .motion(Prop::Size, Motion::Snap)
            .child(at(
                self.aurora_text(mode, lines),
                (CARD_W - TEXT_W) / 2.0 * k,
                y,
            ));
        at(clip, 0.0, CLIP_TOP * k)
    }

    fn aurora_text(&self, mode: Mode, lines: u32) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let tw = l.text_width();
        let dim = if mode == Mode::Cancelled { 0.45 } else { 1.0 };
        let words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(hex(INK))
            .text_halo(hex(MESH_MID).fade(0.45), 8.0 * k)
            .opacity(dim)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.ghost_enter(Transition::fade().opacity(0.0).offset(0.0, 6.0 * k)));
        let paths = aurora_paths(l);
        let words = self.typo_words_in(words, &paths);
        let words = self.handoff_or(words, &l.text_style(), Some(tw), hex(INK), hex(CYAN));
        let editing = self.edit_view.is_some();
        let text = Element::column("text")
            .width(tw)
            .height(lines as f32 * TEXT_LINE * k)
            .child(words)
            // MO-80: the pen over the words while they are being written.
            .children(self.paths_overlays(&paths).into_iter().filter(|_| !editing))
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
        let text = self.signal_marks(text, &self.shown, tw, (0.0, 0.0), hex(INK));
        self.with_hearing(text, &l.text_style(), tw, hex(INK), hex(CYAN))
    }

    /// The status row, centred: a dot, the state word and the detail or
    /// hint, and a thin progress line while a model loads or the final
    /// text is made.
    fn aurora_status(&self, mode: Mode, mode_at: f64, k: f32, w: f32) -> Element<Msg> {
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
            Mode::Listening => (status.state.clone(), status.detail.clone(), hex(RED)),
            _ => (status.state.clone(), status.detail.clone(), hex(VIOLET)),
        };
        let hollow = mode == Mode::Finishing;
        let mark = Element::canvas("dot", mode_at, move |_, r, p| {
            let c = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            if hollow {
                p.stroke(
                    &make::ring(c, 4.2 * k),
                    &StrokeStyle::solid(1.3 * k, hex(INK)),
                );
            } else {
                p.shape(ShapeInstance::circle(
                    c,
                    4.2 * k,
                    Paint::glow(dot.fade(0.5), 4.0 * k, 0.5),
                ));
                p.shape(ShapeInstance::circle(c, 4.2 * k, Paint::solid(dot)));
            }
        })
        .size(12.0 * k, 12.0 * k)
        .motion(Prop::Position, Motion::Snap);
        let words = super::kit::detail_words(&state, &detail);
        let mut row = Element::row("status")
            .height(STATUS_H * k)
            .align_items(AlignItems::CENTER)
            .motion(Prop::Position, Motion::Snap)
            .child(mark)
            .child(
                // Keyed once, not by state: a new key per state faded the
                // old word out under the new one.
                Element::label("state", state.as_str())
                    .text_style(style)
                    .color(hex(INK).fade(0.92))
                    .margin_edges(0.0, 0.0, 0.0, 8.0 * k)
                    .motion(Prop::Position, Motion::Snap),
            )
            .child(
                Element::live_text("detail", &words)
                    .text_style(style)
                    .color(hex(HINT).fade(0.8))
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
                        track: hex(INK).fade(0.18),
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
        Element::row("status-row")
            .width(w)
            .height(STATUS_H * k)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .child(row)
    }
}
