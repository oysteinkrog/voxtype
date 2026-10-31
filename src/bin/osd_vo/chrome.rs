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

use std::f32::consts::TAU;
use std::fmt;

use super::kit::{at, ease_out, hex, mix, Levels, Mode};
use super::*;

use vo::scene::canvas::Painter;
use vo::scene::display::{BackdropRes, Paint, ShapeInstance};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::glass::Glass;
use vo::scene::path::{icons, make, Contour, Outline, StrokeStyle};
use vo_3d::sdf::HeroInside;
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
/// Idle (RD-02): the blob's top bobs `BOB_PX` at `BOB_HZ`, and it sways
/// `SWAY_RAD` either way at `SWAY_HZ`, so its reflections turn at most
/// 5 degrees a second (0.139 rad x 2 pi x 0.1 Hz).
const BOB_PX: f32 = 2.0;
const BOB_HZ: f32 = 0.3;
const SWAY_RAD: f32 = 0.139;
const SWAY_HZ: f32 = 0.1;
/// The frame pace while the popup records room tone (RD-11): the blob's
/// idle bob and sway move under 0.5 px a frame at this pace.
pub(crate) const ROOM_TONE_FPS: f64 = 8.0;
/// How long the send arrow flies once the text lands.
const SEND_SECS: f64 = 0.7;

/// R2-05-a: an aurora inside the chrome, seen through a see-through skin
/// (EG-19). The voice brightens it; in a pause it drifts at its idle
/// brightness. Under reduced motion it stands still and only its
/// brightness follows the voice.
pub(crate) const BREATHING_AURORA: &str = "breathing-aurora";
const AURORA_SKIN: f32 = 0.6;
const AURORA_IDLE: f32 = 0.5;
const AURORA_VOICE: f32 = 1.4;
const AURORA_STEPS: u32 = 10;
/// R2-05-b: fine letter dust rises to the newest word from just under it
/// and fades as it arrives, in `DUST_SECS`; the word keeps a faint halo
/// for `HALO_SECS`. Under reduced motion only the halo.
pub(crate) const TYPEWRITER_WHISPER: &str = "typewriter-whisper";
const DUST_SECS: f64 = 0.4;
const DUST_ALPHA: f32 = 0.35;
const DUST_SCALE: f32 = 0.35;
/// How far under the baseline the dust starts and where it fades out, in
/// design pixels.
const DUST_FROM: f32 = 30.0;
const DUST_TO: f32 = 8.0;
const HALO_SECS: f64 = 0.5;
const HALO_ALPHA: f32 = 0.07;
/// How long a word that opens a new line waits before its whisper.
const LINE_WAIT: f64 = 0.45;
/// R2-05-c: rings on the chrome's dome, one per syllable onset, and one
/// ring per word on a strip of water under the words. At most
/// `MAX_RIPPLES` of each at once. Under reduced motion the dome brightens
/// per onset instead.
pub(crate) const RIPPLE_LENS: &str = "ripple-lens";
const RIPPLE_SECS: f64 = 0.8;
const MAX_RIPPLES: usize = 4;
const RIPPLE_ALPHA: f32 = 0.3;
const POOL_SECS: f64 = 1.0;
const POOL_REACH: f32 = 70.0;
const POOL_ALPHA: f32 = 0.22;
/// The water strip's middle under the last line's box, in design pixels.
const POOL_GAP: f32 = 9.0;
/// Where the dome's rings sit, how far they run and how flat they look,
/// in the blob's radii.
const DOME_UP: f32 = 0.32;
const DOME_REACH: f32 = 0.22;
const DOME_TILT: f32 = 0.3;

/// R2-05-c: the rings alive at `t` for events at `times` (oldest first),
/// each how far it has run (0 to 1) over `life` seconds. At most
/// `MAX_RIPPLES`, the newest.
fn rings(t: f64, times: &[f64], life: f64) -> Vec<(usize, f32)> {
    let alive: Vec<(usize, f32)> = times
        .iter()
        .enumerate()
        .filter(|(_, &at)| t >= at && t - at < life)
        .map(|(i, &at)| (i, ((t - at) / life) as f32))
        .collect();
    let skip = alive.len().saturating_sub(MAX_RIPPLES);
    alive[skip..].to_vec()
}

/// R2-05-b: letter dust `u` of the way (0 to 1): how far under the
/// baseline it is (design pixels) and its alpha.
fn dust(u: f32) -> (f32, f32) {
    let u = u.clamp(0.0, 1.0);
    let rise = DUST_FROM + (DUST_TO - DUST_FROM) * ease_out(u as f64);
    (rise, DUST_ALPHA * (4.0 * u).min(1.0) * (1.0 - u))
}

// The blob's foot ends above the waveform, the words sit between the
// waveform and the status row, and the status row inside the card.
const _: () = assert!(BLOB_Y + 0.18 * BLOB_W / 2.0 < WAVE_Y - WAVE_HALF_H);
const _: () = assert!(BLOB_ROOM > -BLOB_Y + 0.62 * BLOB_W / 2.0);
const _: () = assert!(TEXT_TOP > WAVE_Y + WAVE_HALF_H + 10.0);
const _: () = assert!(TEXT_TOP + MAX_LINES as f32 * TEXT_LINE < STATUS_Y - PROGRESS_GAP - 8.0);
const _: () = assert!(STATUS_Y + STATUS_H / 2.0 < CARD_H - 20.0);
const _: () = assert!(TEXT_X + TEXT_W <= CARD_W - TEXT_X);

/// The words' colour, for the letter effects drawn over them.
pub(crate) fn text_color() -> Color {
    hex(INK)
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
    /// R2-05-a: the aurora inside.
    aurora: bool,
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
    fn frame(&self, t: f64, size: Size) -> HeroFrame {
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
        // Idle (RD-02): the dome rises and falls 2 px at 0.3 Hz; its top
        // stands about 0.55 radii above the middle.
        let reach = 0.55 * size.w / 2.0;
        let bob = BOB_PX * (TAU * BOB_HZ * phase).sin() / reach.max(1.0) * calm;
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
                    ..Env::default()
                }),
            },
            stretch: [1.0, 1.0 + 0.06 * voice + bob, 1.0],
            wobble: (0.12 + 0.5 * voice) * calm,
            phase,
            // A slow sway, never a full turn: the blob keeps its outline.
            spin: SWAY_RAD * (TAU * SWAY_HZ * phase).sin() * calm,
            steps: BLOB_STEPS,
            inside: self.aurora.then(|| {
                let level = if self.mode == Mode::Listening {
                    (self.levels.at(t) * 1.6).min(1.0)
                } else {
                    0.0
                };
                HeroInside {
                    skin: AURORA_SKIN,
                    core: Color::WHITE,
                    core_glow: 0.0,
                    fog_a: hex(CYAN),
                    fog_b: hex(VIOLET),
                    fog: if self.mode == Mode::Cancelled {
                        0.3
                    } else {
                        AURORA_IDLE + AURORA_VOICE * level
                    },
                    fog_scale: 1.6,
                    fog_steps: AURORA_STEPS,
                }
            }),
        }
    }

    fn until(&self) -> f64 {
        // Reduced motion with the aurora: a still shape whose aurora still
        // brightens with the voice.
        if self.moving() || (self.aurora && self.still && self.mode == Mode::Listening) {
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
            aurora: l.on(BREATHING_AURORA),
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
            ))
            .children(self.chrome_dome_rings(mode, s, k));
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
            .children(self.chrome_sculpture(mode, Size::new(w, h), k))
            .children(self.chrome_pool(mode, Size::new(w, h), k))
            .children(self.chrome_whisper(mode, Size::new(w, h), k))
            .child(self.chrome_text(mode, k))
            .children(self.chrome_progress(mode, k, w))
            .child(at(
                self.chrome_status(mode, mode_at, k, w),
                0.0,
                (STATUS_Y - STATUS_H / 2.0) * k,
            ))
    }

    /// The newest word's box in the card, its baseline, and when its
    /// whisper starts.
    fn chrome_newest(&self, k: f32) -> Option<(Rect, f32, f64)> {
        let l = &self.look;
        let style = l.text_style();
        let rows = self.wrap_lines(&self.shown, &style, text_width(l));
        let row = rows.len().min(MAX_LINES as usize).checked_sub(1)?;
        let last = rows.last()?;
        let newest = last.last()?;
        let mut born = self.heard.word(newest.id)?.born;
        // A word that opens a line waits for the line to scroll or grow in.
        if rows.len() > 1 && last.len() == 1 {
            born += LINE_WAIT;
        }
        let (boxes, _) = sounding::word_boxes(&self.measure, last, &style, None);
        let b = boxes.iter().find(|b| b.id == newest.id)?;
        let top = (TEXT_TOP + row as f32 * TEXT_LINE) * k;
        let rect = Rect::new(TEXT_X * k + b.rect.x, top + b.rect.y, b.rect.w, b.rect.h);
        Some((rect, top + b.baseline, born))
    }

    /// R2-05-b: letter dust rising to the newest word from just under it,
    /// and the word's faint halo. Behind the words; under reduced motion
    /// the halo only.
    fn chrome_whisper(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(TYPEWRITER_WHISPER) || mode != Mode::Listening || self.edit_view.is_some() {
            return None;
        }
        let (rect, baseline, born) = self.chrome_newest(k)?;
        if self.extra.now - born >= HALO_SECS {
            return None;
        }
        let still = self.still();
        let word = self.shown.last()?;
        let style = l.text_style();
        let shaped = self.measure.layout(&word.text, &style, None);
        let outline = vo::scene::glyph_morph::WordOutline::new(
            self.measure.fonts(),
            &shaped,
            style.size,
            Point::ZERO,
        );
        // Each letter's ink, centred on its own middle, and that middle's
        // place across the word.
        let letters: Arc<[(f32, Arc<[Contour]>)]> = outline
            .glyphs
            .iter()
            .filter(|g| g.ink.h > 0.0)
            .map(|g| {
                let (cx, cy) = (g.ink.x + g.ink.w / 2.0, g.ink.y + g.ink.h / 2.0);
                let m = Affine::translate(-cx, -cy).then(&Affine::scale(DUST_SCALE));
                let ink: Arc<[Contour]> =
                    g.outline.contours.iter().map(|c| c.transform(&m)).collect();
                (cx, ink)
            })
            .collect();
        let rad = 0.5 * style.size;
        let canvas = Element::canvas("whisper", born + HALO_SECS, move |t, r, p| {
            let age = t - born;
            if age < 0.0 {
                return;
            }
            let halo = HALO_ALPHA * (1.0 - (age / HALO_SECS) as f32).clamp(0.0, 1.0);
            if halo > 0.002 {
                let n = ((rect.w / rad).ceil() as usize).max(1);
                for i in 0..=n {
                    let c = Point::new(
                        r.x + rect.x + rect.w * i as f32 / n as f32,
                        r.y + baseline - 0.3 * style.size,
                    );
                    p.shape(ShapeInstance::circle(
                        c,
                        rad,
                        Paint::glow(hex(CHROME_TINT).fade(halo), rad, 0.5),
                    ));
                }
            }
            if still {
                return;
            }
            for (i, (x, ink)) in letters.iter().enumerate() {
                // Each letter's dust leaves a little after the one before.
                let u = ((age - 0.02 * i as f64) / (DUST_SECS - 0.1)) as f32;
                if !(0.0..1.0).contains(&u) {
                    continue;
                }
                let (below, a) = dust(u);
                let sway = 3.0 * k * ((i as f32 * 2.3).sin());
                let m = Affine::translate(r.x + rect.x + x + sway, r.y + baseline + below * k);
                let moved: Vec<Contour> = ink.iter().map(|c| c.transform(&m)).collect();
                p.fill(&moved, &StrokeStyle::solid(1.0, hex(CHROME_TINT).fade(a)));
            }
        });
        Some(at(canvas.size(size.w, size.h), 0.0, 0.0))
    }

    /// R2-05-c: a ring on the strip of water under the words for each new
    /// word, under its middle. None under reduced motion.
    fn chrome_pool(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
        let l = &self.look;
        if !l.on(RIPPLE_LENS) || mode != Mode::Listening || self.still() || self.edit_view.is_some()
        {
            return None;
        }
        let style = l.text_style();
        let rows = self.wrap_lines(&self.shown, &style, text_width(l));
        let lines = rows.len().clamp(1, MAX_LINES as usize) as f32;
        let last = rows.last()?;
        let (boxes, _) = sounding::word_boxes(&self.measure, last, &style, None);
        let words: Arc<[(f64, f32)]> = boxes
            .iter()
            .filter_map(|b| {
                let born = self.heard.word(b.id)?.born;
                Some((born, TEXT_X * k + b.rect.x + b.rect.w / 2.0))
            })
            .collect();
        let newest = words.iter().map(|w| w.0).fold(f64::NEG_INFINITY, f64::max);
        if self.extra.now - newest >= POOL_SECS {
            return None;
        }
        let y = (TEXT_TOP + lines * TEXT_LINE + POOL_GAP) * k;
        let times: Vec<f64> = words.iter().map(|w| w.0).collect();
        let canvas = Element::canvas("pool", newest + POOL_SECS, move |t, r, p| {
            for (i, u) in rings(t, &times, POOL_SECS) {
                let rx = (6.0 + POOL_REACH * ease_out(u as f64)) * k;
                let a = POOL_ALPHA * (1.0 - u) * (1.0 - u);
                let c = Point::new(r.x + words[i].1, r.y + y);
                let pts: Vec<Point> = (0..48)
                    .map(|j| {
                        let ang = j as f32 / 48.0 * TAU;
                        Point::new(c.x + rx * ang.cos(), c.y + 0.16 * rx * ang.sin())
                    })
                    .collect();
                p.stroke(
                    &Contour::polyline(&pts, true),
                    &StrokeStyle::solid(1.0 * k, hex(CYAN).fade(a)).glow(2.0 * k),
                );
            }
        });
        Some(at(canvas.size(size.w, size.h), 0.0, 0.0))
    }

    /// R2-05-c: rings on the chrome's dome, one per syllable onset; under
    /// reduced motion the dome brightens per onset instead.
    fn chrome_dome_rings(&self, mode: Mode, s: Size, k: f32) -> Option<Element<Msg>> {
        if !self.look.on(RIPPLE_LENS) || mode != Mode::Listening {
            return None;
        }
        let onsets: Vec<f64> = self
            .heard
            .onsets()
            .iter()
            .rev()
            .take(8)
            .rev()
            .copied()
            .collect();
        let newest = *onsets.last()?;
        if self.extra.now - newest >= RIPPLE_SECS {
            return None;
        }
        let still = self.still();
        let r = BLOB_W * k / 2.0;
        let dome = Point::new(s.w / 2.0 - 0.02 * r, (BLOB_ROOM + BLOB_Y) * k - DOME_UP * r);
        let canvas = Element::canvas("dome-rings", newest + RIPPLE_SECS, move |t, rect, p| {
            let c = Point::new(rect.x + dome.x, rect.y + dome.y);
            for (_, u) in rings(t, &onsets, RIPPLE_SECS) {
                let a = RIPPLE_ALPHA * (1.0 - u).powf(1.5);
                if still {
                    let g = 0.12 * r;
                    p.shape(ShapeInstance::circle(
                        c,
                        g,
                        Paint::glow(Color::WHITE.fade(0.6 * a), g, 0.5),
                    ));
                    continue;
                }
                let rx = (0.04 + DOME_REACH * ease_out(u as f64)) * r;
                let pts: Vec<Point> = (0..56)
                    .map(|j| {
                        let ang = j as f32 / 56.0 * TAU;
                        Point::new(c.x + rx * ang.cos(), c.y + DOME_TILT * rx * ang.sin())
                    })
                    .collect();
                p.stroke(
                    &Contour::polyline(&pts, true),
                    &StrokeStyle::solid(0.02 * r * (1.0 - u), Color::WHITE.fade(a)).glow(2.0 * k),
                );
            }
        });
        Some(at(canvas.size(s.w, s.h), 0.0, 0.0))
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
        let words = self.letter_fx_words(self.typo_words(words));
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
            TEXT_X * k,
            TEXT_TOP * k,
        )
    }

    /// MO-88: with `sound-sculpture`, letter bodies behind the words, as
    /// deep as each word was loud; they lie flat once the dictation stops.
    fn chrome_sculpture(&self, mode: Mode, size: Size, k: f32) -> Option<Element<Msg>> {
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

#[cfg(test)]
mod r2_tests {
    use super::*;

    #[test]
    fn at_most_four_rings_and_the_newest() {
        let times: Vec<f64> = (0..10).map(|i| i as f64 * 0.05).collect();
        let alive = rings(0.5, &times, RIPPLE_SECS);
        assert_eq!(alive.len(), MAX_RIPPLES);
        assert_eq!(alive.last().map(|r| r.0), Some(9));
        assert!(alive.iter().all(|&(_, u)| (0.0..1.0).contains(&u)));
    }

    #[test]
    fn rings_follow_the_onsets_only() {
        let times = [1.0, 1.3];
        assert!(rings(0.9, &times, RIPPLE_SECS).is_empty());
        assert_eq!(
            rings(1.2, &times, RIPPLE_SECS),
            rings(1.2, &times, RIPPLE_SECS)
        );
        assert_eq!(rings(1.2, &times, RIPPLE_SECS), vec![(0, 0.25)]);
        assert!(rings(1.3 + RIPPLE_SECS, &times, RIPPLE_SECS).is_empty());
    }

    #[test]
    fn dust_rises_to_just_under_the_word_and_fades() {
        let (start, a0) = dust(0.0);
        let (end, a1) = dust(1.0);
        assert_eq!(start, DUST_FROM);
        assert!((end - DUST_TO).abs() < 1e-3);
        assert_eq!((a0, a1), (0.0, 0.0));
        for i in 0..=20 {
            let (below, a) = dust(i as f32 / 20.0);
            assert!(below >= DUST_TO - 1e-3 && a <= DUST_ALPHA);
        }
    }
}
