//! The `chrome` look of the vo popup, after concept 05, liquid chrome: a
//! dark glass card with a blob of liquid chrome resting on its top edge.
//! The blob is raymarched on the GPU (`vo_3d::hero`) and reflects the card
//! under it; it swells and ripples with the voice and settles in pauses.
//! In the card a dotted waveform with bars in the middle runs across the
//! top, the words sit left-aligned under it and grow downward, and a
//! centred status row runs along the bottom.
//!
//! While the final text is made the status icon turns into a spinner and
//! the bars settle into dots; when the text lands an arrow leaves along the
//! waveform to the right and the status reads the delivery and the word
//! count. A cancel dulls the chrome and dims the words. Under reduced motion
//! the blob rests in one shape, the bars hold still and nothing flies.

use std::fmt;

use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{BackdropRes, Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::Glass;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo_3d::sdf::{HeroPart, HeroParts, PartKind, PartLayer};
use vo_3d::{Env, HeroFrame, HeroLook, HeroShape, HeroSource};

/// Colours sampled from the redesign.
const TINT: u32 = 0x10162c;
const RIM: u32 = 0x8a9ccc;
const INK: u32 = 0xf0f3fa;
const HINT: u32 = 0xaab4cc;
const DIM: u32 = 0x6b7591;
const CYAN: u32 = 0x5ad0ff;
const BLUE: u32 = 0x4a7dff;
const VIOLET: u32 = 0x8a70ff;
/// The chrome's studio: the mirror's tint, the sky above and the floor
/// below, and the light strips.
const CHROME_TINT: u32 = 0xeef1fa;
const CHROME_TOP: u32 = 0x7080a8;
const CHROME_BOTTOM: u32 = 0x10131f;
const CHROME_LIGHT: u32 = 0xffffff;
const CHROME_DULL: u32 = 0x7d8494;

/// The card in the redesign's units at `width_px = 600`; the look scales
/// with the popup's width. `ROOM` is kept round the card for its glow, and
/// `BLOB_ROOM` above it for the blob.
const DESIGN_W: f32 = 600.0;
const CARD_W: f32 = 560.0;
const CARD_H: f32 = 360.0;
const ROOM: f32 = 16.0;
const BLOB_ROOM: f32 = 204.0;
const RADIUS: f32 = 36.0;
/// The blob's box (its width, the hero's diameter) and the middle of the
/// box above the card's top edge. Its shape reaches 0.62 of the radius up
/// and 0.18 down.
const BLOB_W: f32 = 540.0;
const BLOB_Y: f32 = -34.0;
/// March steps per pixel: the blob has no deep folds.
const BLOB_STEPS: u32 = 32;
/// The waveform's middle, half width and half height, and its pitch.
const WAVE_Y: f32 = 64.0;
const WAVE_HALF_W: f32 = 240.0;
const WAVE_HALF_H: f32 = 22.0;
const WAVE_PITCH: f32 = 9.0;
const BAR_W: f32 = 3.0;
const BAR_SECS: f64 = 0.03;
/// Only the middle of the waveform has bars; the rest are dots.
const BAR_REACH: f32 = 0.6;
/// The words: left edge, width, top of the text box, size and pitch.
const TEXT_X: f32 = 40.0;
const TEXT_W: f32 = 480.0;
const TEXT_TOP: f32 = 104.0;
const TEXT_PX: f32 = 26.0;
const TEXT_LINE: f32 = 37.0;
pub(crate) const MAX_LINES: u32 = 4;
/// The status row's middle and height, and the progress line above it.
const STATUS_Y: f32 = 318.0;
const STATUS_H: f32 = 20.0;
const PROGRESS_W: f32 = 220.0;
const PROGRESS_GAP: f32 = 22.0;
/// How long the send arrow flies once the text lands.
const SEND_SECS: f64 = 0.7;

// The blob's foot ends above the waveform, the words sit between the
// waveform and the status row, and the status row inside the card.
const _: () = assert!(BLOB_Y + 0.18 * BLOB_W / 2.0 < WAVE_Y - WAVE_HALF_H);
const _: () = assert!(BLOB_ROOM > -BLOB_Y + 0.62 * BLOB_W / 2.0);
const _: () = assert!(TEXT_TOP > WAVE_Y + WAVE_HALF_H + 10.0);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < STATUS_Y - PROGRESS_GAP - 8.0);
const _: () = assert!(STATUS_Y + STATUS_H / 2.0 < CARD_H - 20.0);
const _: () = assert!(TEXT_X + TEXT_W <= CARD_W - TEXT_X);

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
        ((CARD_W + 2.0 * ROOM) * k).ceil(),
        ((BLOB_ROOM + CARD_H + ROOM) * k).ceil(),
    )
}

/// The card's top in the surface: below the room for the blob.
pub(crate) fn panel_top(look: &Look) -> f32 {
    BLOB_ROOM * scale(look)
}

/// The middle of the newest line below the surface's top. The words grow
/// downward from the top of the text box.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (BLOB_ROOM + TEXT_TOP + (lines.clamp(1, MAX_LINES) as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// The liquid chrome blob: a high dome between two lower shoulders on a
/// flat foot, with a thin drip and a bead at each end.
#[derive(Clone)]
struct Blob {
    levels: Levels,
    shown_at: f64,
    mode: Mode,
    mode_at: f64,
    still: bool,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("mode", &self.mode)
            .field("still", &self.still)
            .finish()
    }
}

impl Blob {
    /// How long the blob takes to settle once the words end.
    const SETTLE_SECS: f64 = 0.8;

    fn moving(&self) -> bool {
        !self.still && matches!(self.mode, Mode::Loading | Mode::Listening | Mode::Finishing)
    }

    fn parts(swell: f32) -> HeroParts {
        let part = |kind, at| HeroPart::new(kind, at, PartLayer::Skin);
        let ellipsoid = |radii| PartKind::Ellipsoid { radii };
        let parts = [
            part(ellipsoid([0.36, 0.32 * swell, 0.34]), [-0.02, -0.2, 0.0]),
            part(ellipsoid([0.3, 0.17, 0.28]), [-0.4, -0.02, 0.0]),
            part(ellipsoid([0.3, 0.18, 0.28]), [0.38, -0.03, 0.0]),
            part(ellipsoid([0.78, 0.07, 0.3]), [0.0, 0.08, 0.0]),
            part(
                PartKind::Capsule {
                    to: [0.52, -0.1, 0.0],
                    radius: 0.035,
                },
                [-0.7, 0.13, 0.0],
            ),
            part(PartKind::Sphere { radius: 0.06 }, [-0.93, 0.13, 0.0]).bobbing(0.015),
            part(
                PartKind::Capsule {
                    to: [-0.52, -0.1, 0.0],
                    radius: 0.035,
                },
                [0.7, 0.12, 0.0],
            ),
            part(PartKind::Sphere { radius: 0.065 }, [0.94, 0.11, 0.0]).bobbing(0.015),
        ];
        HeroParts::new(&parts, 0.12).expect("eight parts")
    }
}

impl HeroSource for Blob {
    fn frame(&self, t: f64, _size: Size) -> HeroFrame {
        // Reduced motion: one resting shape. Once the words end the ripples
        // ease out and the shape holds where it settled.
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
            (self.levels.at(t) * 1.6).min(1.0)
        } else {
            0.0
        };
        let tint = if self.mode == Mode::Cancelled {
            hex(CHROME_DULL)
        } else {
            hex(CHROME_TINT)
        };
        HeroFrame {
            shape: HeroShape::Parts(Self::parts(1.0 + 0.08 * voice)),
            look: HeroLook::Chrome {
                tint,
                top: hex(CHROME_TOP),
                bottom: hex(CHROME_BOTTOM),
                light: hex(CHROME_LIGHT),
                env: Some(Env {
                    reach: 2.5,
                    strength: 0.6,
                    key: None,
                }),
            },
            stretch: [1.0, 1.0 + 0.06 * voice, 1.0],
            wobble: (0.12 + 0.5 * voice) * calm,
            phase,
            // A slow sway, never a full turn: the blob keeps its outline.
            spin: 0.12 * (0.35 * phase).sin() * calm,
            steps: BLOB_STEPS,
            inside: None,
        }
    }

    fn until(&self) -> f64 {
        if self.moving() {
            f64::INFINITY
        } else {
            self.mode_at + Self::SETTLE_SECS + 0.05
        }
    }
}

/// The waveform: a row of dots across the card with mirrored bars in the
/// middle that follow the voice, cyan in the middle and violet outward. When
/// the text lands an arrow leaves along it to the right.
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

    fn draw(&self, t: f64, r: Rect, p: &mut Painter) {
        let k = self.k;
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let n = (WAVE_HALF_W / WAVE_PITCH).floor() as i32;
        let settle = match self.mode {
            Mode::Finishing | Mode::Done | Mode::Cancelled if !self.still => {
                ease_out((t - self.mode_at) / 0.5)
            }
            Mode::Finishing | Mode::Done | Mode::Cancelled => 1.0,
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
        let sweep = (((t - self.shown_at) / 1.4).fract()) as f32 * 2.0 - 1.0;
        for i in -n..=n {
            let reach = i.unsigned_abs() as f32 / n as f32;
            let x = cx + i as f32 * WAVE_PITCH * k;
            let c = if reach < 0.4 {
                mix(hex(CYAN), hex(BLUE), reach / 0.4)
            } else {
                mix(hex(BLUE), hex(VIOLET), (reach - 0.4) / 0.6)
            }
            .fade(dim);
            let dot = |p: &mut Painter, lift: f32| {
                p.shape(ShapeInstance::circle(
                    Point::new(x, cy),
                    (1.3 + lift) * k,
                    Paint::solid(c.fade((0.55 + 0.45 * lift) * (1.0 - 0.5 * reach))),
                ));
            };
            if self.mode == Mode::Loading {
                // A soft light runs along the dots while the model loads.
                let near = if self.still {
                    0.0
                } else {
                    (1.0 - ((i as f32 / n as f32) - sweep).abs() * 5.0).clamp(0.0, 1.0)
                };
                dot(p, near);
                continue;
            }
            if reach > BAR_REACH {
                dot(p, 0.0);
                continue;
            }
            let level = if self.still {
                // Reduced motion: a fixed low shape, not the voice.
                0.25
            } else {
                self.levels.at(t_eff - i.unsigned_abs() as f64 * BAR_SECS)
            };
            let taper = (1.0 - (reach / BAR_REACH).powf(1.8)).max(0.0);
            let half = (level * 1.9).min(1.0) * taper * WAVE_HALF_H * k * (1.0 - settle);
            if half < 2.0 * k {
                dot(p, 0.3);
                continue;
            }
            let bar = Rect::new(x - BAR_W * k / 2.0, cy - half, BAR_W * k, 2.0 * half);
            p.shape(ShapeInstance::rrect(
                RRect::new(bar, BAR_W * k / 2.0),
                Paint::glow(c.fade(0.45), 6.0 * k, 0.6),
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
        if self.mode == Mode::Done {
            self.send(t, Point::new(cx + WAVE_HALF_W * k, cy), p);
        }
    }

    /// The arrow that carries the text away: it runs out along the waveform
    /// past its right end with a trail, and fades as it leaves.
    fn send(&self, t: f64, end: Point, p: &mut Painter) {
        let k = self.k;
        let s = if self.still {
            1.0
        } else {
            ((t - self.mode_at) / SEND_SECS).clamp(0.0, 1.0) as f32
        };
        let go = ease_out(s as f64);
        let x = end.x - 60.0 * k + 70.0 * k * go;
        let fade = if self.still { 1.0 } else { 1.0 - s.powi(3) };
        if fade <= 0.0 {
            return;
        }
        let tail = 70.0 * k * (0.3 + 0.7 * go);
        p.shape(ShapeInstance::rrect(
            RRect::new(Rect::new(x - tail, end.y - 0.8 * k, tail, 1.6 * k), 0.8 * k),
            Paint::linear(
                Point::new(x - tail, end.y),
                hex(CYAN).fade(0.0),
                Point::new(x, end.y),
                hex(CYAN).fade(0.8 * fade),
            ),
        ));
        let head = 9.0 * k;
        let arrow = Contour::polyline(
            &[
                Point::new(x + head, end.y),
                Point::new(x - head * 0.6, end.y - head * 0.75),
                Point::new(x - head * 0.2, end.y),
                Point::new(x - head * 0.6, end.y + head * 0.75),
            ],
            true,
        );
        p.fill(
            [&arrow],
            &StrokeStyle::solid(1.0, hex(INK).fade(fade))
                .linear(
                    Point::new(x - head, end.y - head),
                    hex(INK).fade(fade),
                    Point::new(x + head, end.y + head),
                    hex(CYAN).fade(fade),
                )
                .glow(6.0 * k),
        );
    }
}

impl Popup {
    pub(crate) fn chrome_view(&self) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let s = surface(l);
        let root = Element::column("root").size(s.w, s.h);
        if self.closing {
            return root;
        }
        let mode = self.kit_mode();
        let mode_at = self.kit_mode_at(mode);
        let card = at(self.chrome_card(mode, mode_at), ROOM * k, BLOB_ROOM * k);
        // The blob's layer holds what lies under it (the card), so the
        // chrome reflects the popup itself.
        let d = BLOB_W * k;
        let blob = Blob {
            levels: self.kit_raw_levels(),
            shown_at: self.shown_at,
            mode,
            mode_at,
            still: self.still(),
        };
        let blob = Element::layer("blob-layer")
            .size(s.w, s.h)
            .backdrop(BackdropRes::Half, Color::TRANSPARENT)
            .presence(
                Transition::fade().offset(0.0, 10.0).scale(0.96),
                Transition::fade().scale(0.98),
            )
            .child(at(
                Element::gpu("blob", vo_3d::hero(blob)).size(d, d),
                (s.w - d) / 2.0,
                (BLOB_ROOM + BLOB_Y) * k - d / 2.0,
            ));
        root.child(card).child(at(blob, 0.0, 0.0))
    }

    /// The dark glass card with everything on it.
    fn chrome_card(&self, mode: Mode, mode_at: f64) -> Element<Msg> {
        let l = &self.look;
        let k = scale(l);
        let still = self.still();
        let (w, h) = (CARD_W * k, CARD_H * k);
        let exit = if mode == Mode::Cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let glass = Glass {
            accent: hex(CYAN),
            rim: 1.0,
            bevel: 12.0 * k,
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
        let wave_box = Size::new(
            2.0 * (WAVE_HALF_W + 60.0) * k,
            2.0 * (WAVE_HALF_H + 12.0) * k,
        );
        let until = wave.until();
        Element::panel("card")
            .size(w, h)
            .radius(RADIUS * k)
            .glass(glass)
            .border(Color::hex(RIM, 0.45), 1.0 * k)
            .glow(hex(BLUE).fade(0.18), 10.0 * k, 0.6)
            .rim(Color::hex(RIM, 0.6), 1.4 * k, 0.2, -0.375, 0.0)
            .presence(Transition::fade().offset(0.0, 8.0).scale(0.98), exit)
            .child(at(
                Element::canvas("wave", until, move |t, r, p| wave.draw(t, r, p))
                    .size(wave_box.w, wave_box.h),
                (w - wave_box.w) / 2.0,
                WAVE_Y * k - wave_box.h / 2.0,
            ))
            .children(self.limit_line(
                Point::new(TEXT_X * k, TEXT_TOP * k),
                Size::new(w, h),
                hex(INK).fade(0.55),
                hex(CYAN),
            ))
            .child(self.chrome_text(mode, k))
            .children(self.chrome_progress(mode, k, w))
            .child(at(
                self.chrome_status(mode, mode_at, k, w),
                0.0,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The words, left-aligned, growing downward from the top of the box.
    fn chrome_text(&self, mode: Mode, k: f32) -> Element<Msg> {
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
        let text = self.signal_marks(text, &self.shown, tw, (0.0, 0.0), hex(INK));
        at(
            self.with_hearing(text, &l.text_style(), tw, hex(INK), hex(CYAN)),
            TEXT_X * k,
            TEXT_TOP * k,
        )
    }

    /// A thin line over the status while a model loads or the final text
    /// is made.
    fn chrome_progress(&self, mode: Mode, k: f32, w: f32) -> Option<Element<Msg>> {
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

    /// The status row, centred: an icon (a microphone, a spinner while the
    /// final text is made, a check, a cross), the state word and the detail.
    fn chrome_status(&self, mode: Mode, mode_at: f64, k: f32, w: f32) -> Element<Msg> {
        let l = &self.look;
        let style = TextStyle::new(sans(l), 15.0 * k, STATUS_H * k);
        let hint = TextStyle::new(sans(l), 14.0 * k, STATUS_H * k);
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
                    let rad = 6.5 * k;
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
                Mode::Done => glyph(
                    Outline::new(&icons::check(), 0.05),
                    16.0 * k,
                    hex(CYAN),
                    grow,
                ),
                Mode::Cancelled => glyph(Outline::new(&icons::x(), 0.05), 14.0 * k, hex(DIM), grow),
                Mode::Listening => {
                    glyph(Outline::new(&icons::mic(), 0.05), 18.0 * k, hex(BLUE), 1.0)
                }
            };
            p.outline(&o, &style);
        })
        .size(20.0 * k, 20.0 * k);
        let words = super::kit::detail_words(&state, &detail);
        let (state_c, detail_c) = match mode {
            Mode::Done => (hex(CYAN), hex(CYAN)),
            Mode::Cancelled => (hex(HINT), hex(DIM)),
            _ => (hex(HINT), mix(hex(DIM), hex(HINT), 0.5)),
        };
        let row = Element::row("status")
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
            );
        Element::row("status-row")
            .width(w)
            .height(STATUS_H * k)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .child(row)
    }
}
