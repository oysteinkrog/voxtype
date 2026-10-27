//! The `particles` look of the vo popup, after mock-up 09 "Particle words": a dark navy
//! glass card, a swirl of dotted light round a round microphone button at its top, and
//! large centred words that form out of particles.
//!
//! Draw order, back to front: the card; the swirl as light (five dotted rings that orbit
//! the button, dotted wave strands that flow out to both sides, drifting dust, and dust off
//! the newest word); the button; the words; the status row, the progress bar while the
//! final text is made, and the "Pasted into" chip once it is done.
//!
//! The swirl follows the voice. Its sources read the level from the smoothed levels the
//! popup already keeps, with the time of the newest one, so the GPU can ask for the level
//! at any tick time and no state is kept here.
//!
//! The design is drawn for a 600 px wide card and scales with `[osd] width_px`. The card is
//! taller than the swiss popup: 225 px with one line of words, 385 px with five, plus room
//! above it for the swirl.

use super::*;

use std::sync::OnceLock;

use vo::scene::display::BlendMode;
use vo::scene::emitter::{
    ColorBy, Emitter, EmitterLook, EmitterSim, EmitterSource, Shape, Shock, SpriteStyle,
    WordParticles,
};
use vo::scene::geom::Point;
use vo::scene::path::{icons, make, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Width the design is drawn for; everything scales with `look.width / DESIGN_W`.
const DESIGN_W: f32 = 600.0;
/// Room round the card for the swirl: above, at the sides, below.
const ROOM_TOP: f32 = 46.0;
const ROOM_SIDE: f32 = 14.0;
const ROOM_BOTTOM: f32 = 14.0;
const CARD_RADIUS: f32 = 22.0;
/// The button's centre below the card's top, and its radius.
const RING_Y: f32 = 54.0;
const DISC_R: f32 = 29.0;
/// The rings of the swirl: radius, spin direction, squash, tilt in radians. Squashed and
/// tilted apart, they cross like the loops of a spirograph.
const RINGS: [(f32, f32, f32, f32); 5] = [
    (56.0, 1.0, 0.86, 0.3),
    (63.0, -1.0, 0.78, -0.7),
    (71.0, 1.0, 0.92, 1.25),
    (79.0, -1.0, 0.82, 2.2),
    (88.0, 1.0, 0.95, -1.35),
];
const RING_PARTICLES: u32 = 320;
const DUST_PARTICLES: u32 = 320;
/// Wave strands each side, and where they start and end, from the button's centre.
const STRANDS: usize = 6;
const STRAND_FROM: f32 = 36.0;
const STRAND_TO: f32 = 292.0;
/// The words: top of the first line below the card's top, width, size, line height.
const TEXT_TOP: f32 = 130.0;
const TEXT_W: f32 = 520.0;
const TEXT_PX: f32 = 30.0;
const TEXT_LINE: f32 = 40.0;
/// From the last line's bottom to the middle of the status row, and on to the card's bottom.
const STATUS_GAP: f32 = 32.0;
const STATUS_BOTTOM: f32 = 23.0;
const STATUS_PX: f32 = 13.5;
/// While finishing, the words move up and a progress bar sits between them and the status.
const FINISH_LIFT: f32 = 14.0;
const BAR_W: f32 = 340.0;
const BAR_H: f32 = 4.0;
const BAR_GAP: f32 = 27.0;
/// Once done, the words move up more and the chip replaces the status row.
const DONE_LIFT: f32 = 30.0;
const CHIP_W: f32 = 332.0;
const CHIP_H: f32 = 45.0;
const CHIP_GAP: f32 = 28.0;
const CHIP_BOTTOM: f32 = 20.0;
/// Seconds for the swirl to open, and for the mic to become a check.
const OPEN_SECS: f64 = 0.45;
const MORPH_SECS: f64 = 0.35;

const CYAN: u32 = 0x2fc8ff;
const BLUE: u32 = 0x3a5cff;
const VIOLET: u32 = 0xa855ff;
const INK: u32 = 0xf2f5ff;
const CARD_TOP: u32 = 0x0c1a3e;
const CARD_BOTTOM: u32 = 0x081130;
const EDGE: u32 = 0x3a5ad6;
const MIC: u32 = 0x8ec6ff;
const HINT: u32 = 0x86a8ff;
const LABEL: u32 = 0xd9def0;
const GREEN: u32 = 0x2fd27a;

fn hex(c: u32) -> Color {
    Color::hex(c, 1.0)
}

fn ease_out(k: f64) -> f64 {
    1.0 - (1.0 - k.clamp(0.0, 1.0)).powi(3)
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

struct Fonts {
    text: u32,
    ui: u32,
}

/// Google Sans Flex (OFL) for the words and the status, through fontconfig; the Plasma
/// font when it is not installed.
fn fonts(reg: &FontRegistry) -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let add = |pattern: &str| reg.add_system(pattern).unwrap_or(SANS);
        Fonts {
            text: add("Google Sans Flex:medium"),
            ui: add("Google Sans Flex"),
        }
    })
}

fn scale(look: &Look) -> f32 {
    look.width / DESIGN_W
}

pub(crate) fn text_style(look: &Look) -> TextStyle {
    let s = scale(look);
    TextStyle::new(fonts(&look.fonts).text, TEXT_PX * s, TEXT_LINE * s).centered()
}

pub(crate) fn text_width(look: &Look) -> f32 {
    TEXT_W * scale(look)
}

/// The middle of the newest line of words below the surface's top while listening, with
/// `lines` lines.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> f32 {
    (ROOM_TOP + TEXT_TOP + (lines as f32 - 0.5) * TEXT_LINE) * scale(look)
}

/// Where the dictation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Listening,
    Finishing,
    Done,
}

/// The card's height in design pixels with `lines` lines of words.
fn card_h(lines: u32, phase: Phase) -> f32 {
    let words = TEXT_TOP + lines as f32 * TEXT_LINE;
    match phase {
        Phase::Listening => words + STATUS_GAP + STATUS_BOTTOM,
        Phase::Finishing => words - FINISH_LIFT + BAR_GAP + 22.0 + STATUS_BOTTOM,
        Phase::Done => words - DONE_LIFT + CHIP_GAP + CHIP_H / 2.0 + CHIP_BOTTOM,
    }
}

/// The voice level over time, from the popup's smoothed levels: pure in `t`.
#[derive(Clone, Debug)]
struct Voice {
    levels: Arc<[f32]>,
    /// When the newest level reaches the display.
    newest_at: f64,
    /// From the stop on the level dies away.
    stop_at: Option<f64>,
}

impl Voice {
    fn raw(&self, t: f64) -> f32 {
        let n = self.levels.len();
        if n == 0 {
            return 0.0;
        }
        let i = (n - 1) as f64 - (self.newest_at - t) * FRAME_HZ as f64;
        if i <= 0.0 {
            return if i < -1.0 { 0.0 } else { self.levels[0] };
        }
        if i >= (n - 1) as f64 {
            return self.levels[n - 1];
        }
        let (a, k) = (i.floor() as usize, (i - i.floor()) as f32);
        self.levels[a] + (self.levels[a + 1] - self.levels[a]) * k
    }

    fn at(&self, t: f64) -> f32 {
        let l = match self.stop_at {
            Some(s) if t > s => self.raw(s) * (-(t - s) as f32 / 0.25).exp(),
            _ => self.raw(t),
        };
        (l * 1.4).clamp(0.0, 1.2)
    }
}

/// What the swirl's sources share.
#[derive(Clone, Debug)]
struct Swirl {
    voice: Voice,
    /// The button's centre relative to the centre of the emitters' box.
    origin: Point,
    s: f32,
    shown_at: f64,
    finish_at: Option<f64>,
    final_at: Option<f64>,
    /// 1, or less while the live model loads.
    dim: f32,
}

impl Swirl {
    fn open(&self, t: f64) -> f32 {
        ease_out((t - self.shown_at) / OPEN_SECS) as f32
    }

    fn live(&self, t: f64) -> bool {
        self.finish_at.is_none_or(|f| t < f) || self.final_at.is_some_and(|f| t >= f)
    }

    /// 1 while the final text is made, eased in and out.
    fn calm(&self, t: f64) -> f32 {
        let Some(f) = self.finish_at else { return 0.0 };
        let into = ease_out((t - f) / 0.4);
        let out = self.final_at.map_or(0.0, |d| ease_out((t - d) / 0.5));
        (into * (1.0 - out)) as f32
    }

    /// The final text's flare: 1 when it lands, gone after 0.8 s.
    fn flare(&self, t: f64) -> f32 {
        self.final_at
            .filter(|&f| t >= f)
            .map_or(0.0, |f| (1.0 - (t - f) / 0.8).max(0.0) as f32)
    }

    fn level(&self, t: f64) -> f32 {
        if self.finish_at.is_some_and(|f| t >= f) {
            0.0
        } else {
            self.voice.at(t)
        }
    }
}

/// One dotted ring round the button, as GPU particles.
#[derive(Debug)]
struct Ring {
    swirl: Swirl,
    radius: f32,
    spin: f32,
    squash: f32,
    tilt: f32,
}

impl EmitterSource for Ring {
    fn sim(&self, t: f64) -> EmitterSim {
        let w = &self.swirl;
        let (l, calm) = (w.level(t), w.calm(t));
        let s = w.s;
        // While the final text is made the rings close in round the button.
        let r = self.radius * s * (0.6 + 0.4 * w.open(t)) * (1.0 + 0.07 * l) * (1.0 - 0.3 * calm);
        let shock = w.final_at.filter(|&f| t >= f).map(|f| Shock {
            age: (t - f) as f32,
            speed: r * 2.5,
            width: r * 0.2,
            strength: r * 6.0,
        });
        EmitterSim {
            swirl: self.spin * (0.25 + 0.9 * l) * (1.0 - 0.8 * calm),
            spawn_speed: (2.0 + 12.0 * l) * s,
            noise: (3.0 + 16.0 * l) * s * (1.0 - 0.7 * calm),
            noise_scale: 3.0 / r.max(1.0),
            life: 1.7,
            life_jitter: 0.5,
            emit: 1.0,
            shock,
            level: l,
            ..EmitterSim::ring(r, 0.006 + 0.03 * l)
        }
    }

    fn look(&self, t: f64) -> EmitterLook {
        let w = &self.swirl;
        let (l, calm, flare) = (w.level(t), w.calm(t), w.flare(t));
        let (sn, cs) = self.tilt.sin_cos();
        let q = self.squash;
        let live = if w.live(t) { 1.0 } else { 0.0 };
        EmitterLook {
            origin: w.origin,
            rotation: [[cs, -sn * q, 0.0], [sn, cs * q, 0.0], [0.0, 0.0, 1.0]],
            size: 0.9 * w.s * (1.0 + 0.2 * l),
            brightness: w.open(t)
                * w.dim
                * ((0.3 + 0.25 * l) * live + 0.3 * (1.0 - live) + 0.5 * flare - 0.05 * calm),
            // Cyan on the left to violet on the right, as drawn.
            ramp: [hex(CYAN), hex(BLUE), hex(VIOLET)],
            color_by: ColorBy::X {
                half: self.radius * w.s,
            },
            style: SpriteStyle::Spark {
                accents: 0.02,
                gain: 1.0,
                twinkle: 0.2,
            },
            ..EmitterLook::default()
        }
    }
}

/// Sparse dust drifting through the band of the strands.
#[derive(Debug)]
struct Dust {
    swirl: Swirl,
}

impl EmitterSource for Dust {
    fn sim(&self, t: f64) -> EmitterSim {
        let w = &self.swirl;
        let l = w.level(t);
        EmitterSim {
            shape: Shape::Box {
                half: [STRAND_TO * w.s, 26.0 * w.s, 0.0],
            },
            emit: 1.0,
            life: 3.0,
            life_jitter: 0.6,
            spawn_speed: 3.0 * w.s,
            noise: (8.0 + 24.0 * l) * w.s,
            noise_scale: 0.02,
            noise_speed: 0.4,
            drag: 1.5,
            level: l,
            ..EmitterSim::default()
        }
    }

    fn look(&self, t: f64) -> EmitterLook {
        let w = &self.swirl;
        let l = w.level(t);
        EmitterLook {
            origin: w.origin,
            size: 0.8 * w.s,
            brightness: w.open(t) * w.dim * (0.15 + 0.2 * l) * (1.0 - 0.5 * w.calm(t)),
            ramp: [hex(CYAN), hex(BLUE), hex(VIOLET)],
            color_by: ColorBy::X {
                half: STRAND_TO * w.s,
            },
            style: SpriteStyle::Spark {
                accents: 0.0,
                gain: 1.0,
                twinkle: 0.3,
            },
            ..EmitterLook::default()
        }
    }
}

/// Dust that leaves the newest word's end and drifts off to the right, as in the mock-up.
#[derive(Debug)]
struct Trail {
    /// The word's right end, mid x-height, relative to the centre of the emitter's box.
    origin: Point,
    s: f32,
}

impl EmitterSource for Trail {
    fn sim(&self, _: f64) -> EmitterSim {
        EmitterSim {
            shape: Shape::Line {
                half_length: 9.0 * self.s,
                angle: std::f32::consts::FRAC_PI_2,
            },
            emit: 0.8,
            life: 1.1,
            life_jitter: 0.5,
            spawn_speed: 10.0 * self.s,
            gravity: [42.0 * self.s, -4.0 * self.s, 0.0],
            noise: 30.0 * self.s,
            noise_scale: 0.05,
            drag: 1.2,
            ..EmitterSim::default()
        }
    }

    fn look(&self, _: f64) -> EmitterLook {
        EmitterLook {
            origin: self.origin,
            size: 0.9 * self.s,
            brightness: 0.7,
            ramp: [
                Color::hex(0x8fc4ff, 1.0),
                Color::hex(BLUE, 0.9),
                Color::hex(VIOLET, 0.0),
            ],
            style: SpriteStyle::SPARK,
            ..EmitterLook::default()
        }
    }
}

/// The dotted wave strands on one side of the ring, as a path emitter: particles flow out
/// along six swaying strands whose height follows the voice.
#[derive(Debug)]
struct Wing {
    swirl: Swirl,
    /// From the ring outward, in emitter space.
    path: Arc<[Point]>,
}

impl Wing {
    fn new(swirl: Swirl, side: f32) -> Wing {
        let s = swirl.s;
        let path: Arc<[Point]> = (0..=32)
            .map(|i| {
                Point::new(
                    side * (STRAND_FROM + (STRAND_TO - STRAND_FROM) * i as f32 / 32.0) * s,
                    0.0,
                )
            })
            .collect();
        Wing { swirl, path }
    }
}

impl EmitterSource for Wing {
    fn sim(&self, t: f64) -> EmitterSim {
        let w = &self.swirl;
        let s = w.s;
        let (l, calm, flare) = (w.level(t), w.calm(t), w.flare(t));
        let quiet = if w.dim < 1.0 { 0.2 } else { 1.0 };
        EmitterSim {
            shape: Shape::Path {
                strands: STRANDS as u32,
                sway: (24.0 + 26.0 * l + 18.0 * flare)
                    * (1.0 - 0.45 * calm)
                    * quiet
                    * w.open(t)
                    * s,
                wavelength: 130.0 * s,
                wave_speed: 0.35 + 0.4 * l,
                width: 1.5 * s,
            },
            spawn_speed: (50.0 + 40.0 * l) * s,
            emit: 1.0,
            life: 6.0,
            level: l,
            ..EmitterSim::default()
        }
    }

    fn look(&self, t: f64) -> EmitterLook {
        let w = &self.swirl;
        let l = w.level(t);
        EmitterLook {
            origin: w.origin,
            size: 0.8 * w.s,
            brightness: w.open(t) * w.dim * (0.35 + 0.3 * l),
            ramp: [hex(CYAN), hex(BLUE), hex(VIOLET)],
            color_by: ColorBy::X {
                half: STRAND_TO * w.s,
            },
            style: SpriteStyle::Spark {
                accents: 0.01,
                gain: 1.0,
                twinkle: 0.25,
            },
            ..EmitterLook::default()
        }
    }

    fn path(&self) -> Option<Arc<[Point]>> {
        Some(self.path.clone())
    }
}

impl Popup {
    pub(crate) fn particles_surface(look: &Look) -> Size {
        let s = scale(look);
        let h = ROOM_TOP + card_h(swiss::MAX_LINES as u32, Phase::Finishing) + ROOM_BOTTOM;
        Size::new((look.width + 2.0 * ROOM_SIDE * s).ceil(), (h * s).ceil())
    }

    fn particles_phase(&self) -> Phase {
        if self.extra.final_at.is_some() {
            Phase::Done
        } else if self.extra.finish_at.is_some() || self.status().finishing {
            Phase::Finishing
        } else {
            Phase::Listening
        }
    }

    pub(crate) fn particles_view(&self) -> Element<Msg> {
        let l = &self.look;
        let s = scale(l);
        let surface = Self::particles_surface(l);
        let root = Element::column("root").size(surface.w, surface.h);
        if self.closing {
            return root;
        }
        let phase = self.particles_phase();
        let (x0, y0) = (ROOM_SIDE * s, ROOM_TOP * s);
        let card_h = card_h(self.lines, phase) * s;
        let swirl = Swirl {
            voice: Voice {
                levels: self.wave.clone(),
                newest_at: self.newest_at,
                stop_at: self.extra.finish_at,
            },
            origin: Point::new(
                x0 + l.width / 2.0 - surface.w / 2.0,
                y0 + RING_Y * s - surface.h / 2.0,
            ),
            s,
            shown_at: self.shown_at,
            finish_at: self.extra.finish_at,
            final_at: self.extra.final_at,
            dim: if self.status().loading { 0.25 } else { 1.0 },
        };
        root.child(at(self.particles_card(card_h), x0, y0))
            .child(at(self.particles_swirl(&swirl, surface), 0.0, 0.0))
            .child(at(
                self.particles_button(phase),
                x0 + l.width / 2.0 - DISC_R * s,
                y0 + (RING_Y - DISC_R) * s,
            ))
            .child(self.particles_trail(phase, x0, y0, surface))
            .child(self.particles_words(phase, x0, y0))
            .child(self.particles_bottom(phase, x0, y0))
    }

    /// The navy glass card with its thin blue-violet edge.
    fn particles_card(&self, h: f32) -> Element<Msg> {
        let l = &self.look;
        let s = scale(l);
        let cancelled = self.extra.final_at.is_none();
        let exit = if cancelled {
            Transition::fade().offset(0.0, 6.0).scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        let a = l.background.a.max(0.86);
        Element::panel("card")
            .size(l.width, h)
            .radius(CARD_RADIUS * s)
            .fill_gradient(Color::hex(CARD_TOP, a), Color::hex(CARD_BOTTOM, a))
            .border(Color::hex(EDGE, 0.55), 1.0)
            .rim(Color::hex(VIOLET, 0.35), 1.0, 0.18, -0.4, 0.0)
            .presence(Transition::fade().offset(0.0, 8.0), exit)
    }

    /// The swirl as light: strands, rings and dust, with a bloom.
    fn particles_swirl(&self, swirl: &Swirl, surface: Size) -> Element<Msg> {
        let mut layer = Element::layer("swirl")
            .size(surface.w, surface.h)
            .blend(BlendMode::Add)
            .bloom(
                0.35 + 0.25 * swirl.voice.at(self.extra.now.max(self.shown_at)),
                5.0 * swirl.s,
            )
            .presence(Transition::fade(), Transition::fade());
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let wing = Wing::new(swirl.clone(), side);
            layer = layer.child(at(
                Element::emitter(
                    format!("wing-{i}"),
                    Emitter::new(Arc::new(wing), 1200, 41 + i as u32),
                )
                .size(surface.w, surface.h),
                0.0,
                0.0,
            ));
        }
        for (i, &(radius, spin, squash, tilt)) in RINGS.iter().enumerate() {
            let ring = Ring {
                swirl: swirl.clone(),
                radius,
                spin,
                squash,
                tilt,
            };
            layer = layer.child(at(
                Element::emitter(
                    format!("ring-{i}"),
                    Emitter::new(Arc::new(ring), RING_PARTICLES, 7 + i as u32),
                )
                .size(surface.w, surface.h),
                0.0,
                0.0,
            ));
        }
        let dust = Dust {
            swirl: swirl.clone(),
        };
        layer.child(at(
            Element::emitter("dust", Emitter::new(Arc::new(dust), DUST_PARTICLES, 23))
                .size(surface.w, surface.h),
            0.0,
            0.0,
        ))
    }

    /// The round button: a dark disc with a thin blue rim, the mic in it, which becomes a
    /// check once the recording stops.
    fn particles_button(&self, phase: Phase) -> Element<Msg> {
        static MIC_TO_CHECK: OnceLock<Arc<Morph>> = OnceLock::new();
        let s = scale(&self.look);
        let d = 2.0 * DISC_R * s;
        let morph = MIC_TO_CHECK.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let check = Outline::new(&icons::check(), 0.05);
            Arc::new(Morph::new(&mic, &check, 64))
        });
        let checked = phase != Phase::Listening;
        let icon = 22.0 * s;
        Element::layer("button")
            .size(d, d)
            .presence(Transition::fade().scale(0.8), Transition::fade())
            .child(at(
                Element::shape("disc", ShapeKind::RRect)
                    .size(d, d)
                    .radius(d / 2.0)
                    .fill_gradient(Color::hex(0x13224d, 1.0), Color::hex(0x0b1536, 1.0))
                    .border(Color::hex(EDGE, 0.75), 1.0)
                    .glow(Color::hex(BLUE, 0.5), 8.0 * s, 0.35),
                0.0,
                0.0,
            ))
            .child(at(
                Element::morph(
                    "mic",
                    morph.clone(),
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                    StrokeStyle::solid(1.9, hex(MIC)),
                )
                .size(icon, icon)
                .morph_to(if checked { 1.0 } else { 0.0 })
                .motion(Prop::Morph, Spring::critical_to_98(MORPH_SECS)),
                (d - icon) / 2.0,
                (d - icon) / 2.0,
            ))
    }

    /// The newest word's id and its right end on its baseline, in the text box, as the
    /// live text lays it out.
    fn particles_newest(&self) -> Option<(u32, Point)> {
        let last = self.shown.last()?;
        let l = &self.look;
        let style = text_style(l);
        let width = text_width(l);
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let start = (text.len() - last.text.len()) as u32;
        let mut shaped = self.measure.layout(&text, &style, Some(width));
        shaped.align_in(style.align, width);
        let mut glyphs = shaped
            .glyphs
            .iter()
            .filter(|g| g.cluster >= start && g.cluster != u32::MAX);
        let first = glyphs.next()?;
        let end = glyphs.fold(first.pos.x + first.advance, |x, g| {
            x.max(g.pos.x + g.advance)
        });
        Some((last.id, Point::new(end, first.pos.y)))
    }

    /// The words, centred, forming from particles.
    fn particles_words(&self, phase: Phase, x0: f32, y0: f32) -> Element<Msg> {
        let l = &self.look;
        let s = scale(l);
        let lift = match phase {
            Phase::Listening => 0.0,
            Phase::Finishing => FINISH_LIFT,
            Phase::Done => DONE_LIFT,
        };
        let words = Element::live_text("words", &self.shown)
            .text_style(text_style(l))
            .color(hex(INK))
            .motion(Prop::Opacity, Spring::critical_to_98(0.10))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.0))
            .word_change(Transition::fade().opacity(0.4))
            .word_particles(WordParticles {
                color: Color::hex(0x6fa6ff, 0.95),
                style: SpriteStyle::SPARK,
                dot_area: 5.0,
                // The newest word keeps its last 40 % as dots until the next word or the
                // final text, as "polis" and "hed" in the mock-up.
                hold: if phase == Phase::Done { 0.0 } else { 0.4 },
                ..WordParticles::default()
            })
            .presence(Transition::fade(), Transition::fade().dissolve(1.0));
        at(
            Element::column("text")
                .size(TEXT_W * s, self.lines as f32 * TEXT_LINE * s)
                .child(words),
            x0 + (l.width - TEXT_W * s) / 2.0,
            y0 + (TEXT_TOP - lift) * s,
        )
    }

    /// The dust off the newest word while listening, one pool per word so a new word does
    /// not drag the old dust along.
    fn particles_trail(&self, phase: Phase, x0: f32, y0: f32, surface: Size) -> Element<Msg> {
        let l = &self.look;
        let s = scale(l);
        let layer = Element::layer("trail")
            .size(surface.w, surface.h)
            .blend(BlendMode::Add);
        let Some((id, end)) = self
            .particles_newest()
            .filter(|_| phase == Phase::Listening)
        else {
            return at(layer, 0.0, 0.0);
        };
        let tx = x0 + (l.width - TEXT_W * s) / 2.0;
        let ty = y0 + TEXT_TOP * s;
        let p = Point::new(
            tx + end.x + 2.0 * s - surface.w / 2.0,
            ty + end.y - 0.3 * TEXT_PX * s - surface.h / 2.0,
        );
        let trail = Trail { origin: p, s };
        at(
            layer.child(at(
                Element::emitter(
                    format!("trail-{id}"),
                    Emitter::new(Arc::new(trail), 192, 31 + id),
                )
                .size(surface.w, surface.h)
                .presence(Transition::fade(), Transition::fade()),
                0.0,
                0.0,
            )),
            0.0,
            0.0,
        )
    }

    /// Under the words: the status row while listening, the progress bar and the status
    /// while finishing, the chip once done.
    fn particles_bottom(&self, phase: Phase, x0: f32, y0: f32) -> Element<Msg> {
        let l = &self.look;
        let s = scale(l);
        let words_bottom = TEXT_TOP + self.lines as f32 * TEXT_LINE;
        let col = Element::column("bottom").size(l.width, 0.0);
        match phase {
            Phase::Done => {
                let y = words_bottom - DONE_LIFT + CHIP_GAP - CHIP_H / 2.0;
                at(
                    col.child(at(self.particles_chip(), (l.width - CHIP_W * s) / 2.0, 0.0)),
                    x0,
                    y0 + y * s,
                )
            }
            _ => {
                let lift = if phase == Phase::Finishing {
                    FINISH_LIFT
                } else {
                    0.0
                };
                let mut col = col;
                let status = self.status();
                let mut status_y = words_bottom - lift + STATUS_GAP;
                if self.shows_progress(&status) {
                    let bar_y = words_bottom - lift + BAR_GAP;
                    status_y = bar_y + 22.0;
                    col = col.child(at(
                        self.particles_bar(),
                        (l.width - BAR_W * s) / 2.0,
                        (bar_y - BAR_H / 2.0) * s,
                    ));
                }
                let row = self.particles_status(&status, phase).presence(
                    Transition::fade().offset(0.0, 5.0),
                    Transition::fade().offset(0.0, -5.0),
                );
                let row_h = 20.0 * s;
                at(col.child(at(row, 0.0, status_y * s - row_h / 2.0)), x0, y0)
            }
        }
    }

    /// The progress bar: a cyan to blue line that grows to the fraction.
    fn particles_bar(&self) -> Element<Msg> {
        let s = scale(&self.look);
        let (w, h) = (BAR_W * s, BAR_H * s);
        let line = Arc::new(Outline {
            contours: vec![make::underline(h / 2.0, w - h / 2.0, h / 2.0)],
        });
        let track = StrokeStyle::solid(h, Color::hex(0x1a2a5c, 0.9));
        let fill = StrokeStyle {
            c0: hex(CYAN),
            c1: hex(BLUE),
            ramp: [0.0, w],
            ..StrokeStyle::solid(h, hex(CYAN))
        };
        let fraction = self
            .files
            .fraction
            .map_or(0.15, |f| f.clamp(0.02, 1.0) as f32);
        Element::layer("bar")
            .size(w, h)
            .presence(Transition::fade(), Transition::fade())
            .child(at(
                Element::stroke_path("track", line.clone(), Rect::new(0.0, 0.0, w, h), track)
                    .size(w, h),
                0.0,
                0.0,
            ))
            .child(at(
                Element::stroke_path("fill", line, Rect::new(0.0, 0.0, w, h), fill)
                    .size(w, h)
                    .trim(0.0, fraction)
                    .motion(Prop::Trim, Spring::critical_to_98(0.2))
                    .glow(Color::hex(CYAN, 0.6), 4.0 * s, 0.5),
                0.0,
                0.0,
            ))
    }

    /// "● Listening · tap Right Ctrl to stop", centred; a spinner instead of the dot while
    /// the final text is made.
    fn particles_status(&self, status: &StatusLine, phase: Phase) -> Element<Msg> {
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        let l = &self.look;
        let s = scale(l);
        let ui = fonts(&l.fonts).ui;
        let px = STATUS_PX * s;
        let style = TextStyle::new(ui, px, (px * 1.4).ceil());
        let mut row = Element::row(format!("status-{}", status.state))
            .size(l.width, 20.0 * s)
            .align_items(AlignItems::CENTER)
            .justify(vo::scene::taffy::JustifyContent::CENTER)
            .gap(8.0 * s);
        if phase == Phase::Finishing || status.loading {
            let side = 15.0 * s;
            let ring = RING.get_or_init(|| {
                Arc::new(Outline {
                    contours: vec![make::ring(Point::new(12.0, 12.0), 9.0)],
                })
            });
            row = row.child(
                Element::stroke_path(
                    "spinner",
                    ring.clone(),
                    Rect::new(0.0, 0.0, 24.0, 24.0),
                    StrokeStyle::solid(2.2, Color::hex(BLUE, 0.25)),
                )
                .size(side, side)
                .light_head(0.7, 1.1, hex(CYAN), 0.0)
                .presence(Transition::fade().scale(0.6), Transition::fade()),
            );
        } else {
            let d = 8.0 * s;
            row = row.child(
                Element::shape("dot", ShapeKind::RRect)
                    .size(d, d)
                    .radius(d / 2.0)
                    .fill(hex(0x5fd0ff))
                    .glow(hex(CYAN), 5.0 * s, 0.6)
                    .presence(Transition::fade().scale(0.6), Transition::fade()),
            );
        }
        row = row.child(
            Element::label(format!("state-{}", status.state), status.state.clone())
                .text_style(style)
                .color(hex(LABEL))
                .presence(
                    Transition::fade().offset(0.0, 5.0),
                    Transition::fade().offset(0.0, -5.0),
                ),
        );
        let detail = status.detail.trim().trim_start_matches('·').trim();
        if !detail.is_empty() {
            row = row
                .child(
                    Element::label("sep", "·")
                        .text_style(style)
                        .color(Color::hex(HINT, 0.7)),
                )
                .child(
                    Element::label(format!("detail-{}", status.state), detail)
                        .text_style(style)
                        .color(hex(HINT))
                        .presence(Transition::fade(), Transition::fade()),
                );
        }
        row
    }

    /// The app the text went to, as a name: "org.kde.konsole" gives "Konsole".
    fn particles_app(&self) -> Option<String> {
        let app = &self.files.live.as_ref()?.target.as_ref()?.app;
        let last = app.rsplit(['.', '/']).next().unwrap_or(app);
        let mut c = last.chars();
        let first = c.next()?;
        Some(first.to_uppercase().chain(c).collect())
    }

    /// "✓ Pasted into Konsole  1.2s" in a rounded chip.
    fn particles_chip(&self) -> Element<Msg> {
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        let l = &self.look;
        let s = scale(l);
        let ui = fonts(&l.fonts).ui;
        let px = 14.5 * s;
        let style = TextStyle::new(ui, px, (px * 1.4).ceil());
        let text = match self.particles_app() {
            Some(app) => format!("Pasted into {app}"),
            None => "Pasted".to_string(),
        };
        let took = match (self.extra.finish_at, self.extra.final_at) {
            (Some(a), Some(b)) => format!("{:.1}s", (b - a).max(0.0)),
            _ => String::new(),
        };
        let side = 26.0 * s;
        let view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let ring = RING.get_or_init(|| {
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(12.0, 12.0), 10.5)],
            })
        });
        let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
        let badge = Element::layer("badge")
            .size(side, side)
            .child(at(
                Element::stroke_path(
                    "ring",
                    ring.clone(),
                    view,
                    StrokeStyle::solid(1.8, hex(GREEN)),
                )
                .size(side, side)
                .motion(Prop::Trim, Spring::critical_to_98(0.3))
                .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                0.0,
                0.0,
            ))
            .child(at(
                Element::stroke_path(
                    "check",
                    check.clone(),
                    view,
                    StrokeStyle::solid(2.6, hex(GREEN)),
                )
                .size(side * 0.6, side * 0.6)
                .motion(Prop::Trim, Spring::critical_to_98(0.55))
                .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                side * 0.2,
                side * 0.2,
            ));
        Element::panel("chip")
            .size(CHIP_W * s, CHIP_H * s)
            .radius(CHIP_H * s / 2.0)
            .pad_edges(0.0, 18.0 * s, 0.0, 12.0 * s)
            .direction_row()
            .align_items(AlignItems::CENTER)
            .gap(10.0 * s)
            .fill(Color::hex(0x16234f, 0.85))
            .border(Color::hex(EDGE, 0.35), 1.0)
            .presence(Transition::fade().scale(0.92), Transition::fade())
            .child(badge)
            .child(
                Element::label("pasted", text)
                    .text_style(style)
                    .color(hex(LABEL))
                    .grow(1.0),
            )
            .child(
                Element::label("took", took)
                    .text_style(style)
                    .color(Color::hex(HINT, 0.8)),
            )
    }
}
