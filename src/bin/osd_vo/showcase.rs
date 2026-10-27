//! The `showcase` look of the vo popup: the swiss popup's behaviour with a
//! rich look on top. Palette and geometry come from `voxtype::osd::showcase`.
//!
//! Draw order, back to front: a tilted particle ring that follows the voice
//! and a field of out-of-focus orbs behind the card, the glass card itself
//! (header with dot-matrix clock, ribbon, the matte transcript strip whose
//! words form from particles, status), then the rim line with its running
//! light head and the corner flares in front.
//!
//! The particles and the light head run on the GPU from the frame time. A
//! 30 Hz tick only runs while the popup opens and for the glow pulse after
//! the final text. From the stop on the ambient motion calms and stops.

use super::*;

use std::sync::OnceLock;

use vo::scene::display::BlendMode;
use vo::scene::emitter::{
    tilt_x, Emitter, EmitterLook, EmitterSim, EmitterSource, LevelTrack, Shape, Shock, SpriteStyle,
    WordParticles,
};
use vo::scene::geom::{Point, Rect};
use vo::scene::glass::Glass;
use vo::scene::path::{icons, make, Morph, Outline, StrokeStyle};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};
use vo_dots::{
    assemble, check_cells, Change, DotField, DotFont, DotMeter, DotText, Dots, Look as DotLook,
    Springs, TextStyle as DotStyle,
};
use voxtype::osd::showcase::{
    self as sc, lit_dots, mix, Phase, EMBER, ICE, INK, INK_HIGH, MATTE, SLATE, TEAL,
};

const PAD_TOP: f32 = 18.0;
const PAD_X: f32 = 22.0;
const PAD_BOTTOM: f32 = 16.0;
const ICON: f32 = 18.0;
const RIBBON_MARGIN: f32 = 10.0;
const STRIP_PAD_Y: f32 = 10.0;
const STRIP_PAD_X: f32 = 12.0;
const STATUS_MARGIN_TOP: f32 = 12.0;
const STATUS_DOT: f32 = 8.0;
const METER_DOTS: usize = 20;

fn c(c: OsdColor) -> Color {
    color(c)
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

/// Seconds for the mic to morph into the stop square.
const MORPH_SECS: f64 = 0.35;

/// A cubic ease out, 0 to 1.
fn ease_out(k: f64) -> f64 {
    1.0 - (1.0 - k).powi(3)
}

/// The y of the newest text line's centre from the surface top, with
/// `lines` text lines, for the reading-line placement. The card sits a glow
/// margin in from the surface's top and grows downward.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> Option<f32> {
    Some(
        sc::MARGIN_PX
            + BORDER
            + PAD_TOP
            + Popup::header_h(look)
            + 2.0 * RIBBON_MARGIN
            + sc::RIBBON_PX
            + STRIP_PAD_Y
            + (lines as f32 - 0.5) * look.text_line,
    )
}

/// State only the showcase look keeps.
#[derive(Default)]
pub struct Extra {
    /// Time of the latest event.
    pub now: f64,
    /// When the recording stopped and the final pass began.
    pub finish_at: Option<f64>,
    /// When the final text came.
    pub final_at: Option<f64>,
    pub tick: Option<vo::TimerId>,
    /// The clock and word count as dot-matrix text, made on the first sync.
    dots: Option<DotHeader>,
    /// The voice level over time, eased, for the particle ring.
    level: LevelTrack,
    /// The level last pushed to `level`.
    level_sent: Option<f32>,
    /// The status icon's events for the `type` look, in order.
    pub icon_events: Vec<(super::type_look::IconEvent, f64)>,
    /// Words the engine was unsure of, with when it became sure (`None`
    /// while still unsure), for the `type` look's wavy underlines.
    pub doubtful: std::collections::HashMap<vo_core::words::WordId, Option<f64>>,
}

/// Reach of the card's glow. Its falloff is exp(-d / GLOW_PX), so it is
/// gone well inside the surface's transparent margin.
const GLOW_PX: f32 = 8.0;

/// Orbs in the out-of-focus field around the card.
const ORB_PARTICLES: u32 = 20;

/// Soft orbs at different depths around the card, drawn out of focus by
/// their distance from the focal plane. They drift while listening, brighten
/// a little with the voice and thin out once the recording stops. Pure in `t`.
#[derive(Debug)]
struct OrbField {
    level: LevelTrack,
    /// Half width and height of the field in logical pixels.
    half: [f32; 2],
    /// Field centre relative to the centre of the emitter's box.
    origin: Point,
    shown_at: f64,
    finish_at: Option<f64>,
}

impl OrbField {
    fn live(&self, t: f64) -> bool {
        self.finish_at.is_none_or(|f| t < f)
    }
}

impl EmitterSource for OrbField {
    fn sim(&self, t: f64) -> EmitterSim {
        let live = self.live(t);
        let l = if live {
            self.level.at(t).clamp(0.0, 1.2)
        } else {
            0.0
        };
        EmitterSim {
            shape: Shape::Box {
                half: [self.half[0], self.half[1], 60.0],
            },
            emit: if live { 1.0 } else { 0.3 },
            life: 5.0,
            life_jitter: 0.6,
            spawn_speed: 6.0,
            noise: 14.0 + 30.0 * l,
            noise_scale: 0.01,
            noise_speed: 0.3,
            drag: 2.0,
            level: l,
            ..EmitterSim::default()
        }
    }

    fn look(&self, t: f64) -> EmitterLook {
        let live = self.live(t);
        let l = if live {
            self.level.at(t).clamp(0.0, 1.2)
        } else {
            0.0
        };
        let open = ((t - self.shown_at) / sc::OPEN_SECS).clamp(0.0, 1.0) as f32;
        EmitterLook {
            origin: self.origin,
            size: 3.0,
            brightness: open * (0.2 + 0.15 * l),
            ramp: [c(ICE).fade(0.0), c(TEAL).fade(0.8), c(TEAL).fade(0.0)],
            style: SpriteStyle::Bokeh {
                focus: 0.55,
                aperture: 25.0,
            },
            ..EmitterLook::default()
        }
    }
}

/// Particles in the voice ring.
const RING_PARTICLES: u32 = 1536;

/// The tilted ring of light behind the card's top edge, as GPU particles. It
/// swells and spins faster with the voice, rests once the recording stops,
/// and a shock runs through it when the final text lands. Pure in `t`.
#[derive(Debug)]
struct VoiceRing {
    level: LevelTrack,
    /// Ring centre relative to the centre of the emitter's box.
    origin: Point,
    radius: f32,
    /// Tilt about x in radians; near a quarter turn the ring is a flat ellipse.
    tilt: f32,
    shown_at: f64,
    finish_at: Option<f64>,
    final_at: Option<f64>,
}

impl VoiceRing {
    fn live(&self, t: f64) -> bool {
        self.finish_at.is_none_or(|f| t < f)
    }

    fn level(&self, t: f64) -> f32 {
        if self.live(t) {
            self.level.at(t).clamp(0.0, 1.2)
        } else {
            0.0
        }
    }

    /// The final text's flare at `t`: 1 when it lands, 0 when it is over.
    fn flare(&self, t: f64) -> f32 {
        self.final_at.map_or(0.0, |f| {
            let k = ((t - f) / (2.0 * sc::FINAL_PULSE_SECS)).clamp(0.0, 1.0) as f32;
            if t < f {
                0.0
            } else {
                1.0 - k
            }
        })
    }
}

impl EmitterSource for VoiceRing {
    fn sim(&self, t: f64) -> EmitterSim {
        let l = self.level(t);
        let live = self.live(t);
        let r = self.radius;
        let open = ease_out(((t - self.shown_at) / sc::OPEN_SECS).clamp(0.0, 1.0)) as f32;
        let shock = self.final_at.filter(|&f| t >= f).map(|f| Shock {
            age: (t - f) as f32,
            speed: r * 2.0,
            width: r * 0.15,
            strength: r * 8.0,
        });
        EmitterSim {
            swirl: if live { 0.35 + 1.6 * l } else { 0.04 },
            spawn_speed: 4.0 + 16.0 * l,
            noise: 6.0 + 18.0 * l,
            noise_scale: 3.0 / r.max(1.0),
            life: 1.8 - 0.6 * l,
            emit: if live { 1.0 } else { 0.4 },
            shock,
            level: l,
            ..EmitterSim::ring(
                r * (0.6 + 0.4 * open) * (1.0 + 0.05 * l),
                0.02 * (1.0 + 2.0 * l),
            )
        }
    }

    fn look(&self, t: f64) -> EmitterLook {
        let l = self.level(t);
        let flare = self.flare(t);
        let brightness = if self.live(t) {
            0.18 + 0.25 * l
        } else {
            0.12 + 0.4 * flare
        };
        let mid = if self.live(t) { TEAL } else { SLATE };
        EmitterLook {
            origin: self.origin,
            rotation: tilt_x(self.tilt),
            size: 0.8 + 0.3 * l,
            brightness,
            ramp: [
                c(mix(mid, ICE, 0.6 + 0.4 * flare)),
                c(mid).fade(0.85),
                c(mid).fade(0.0),
            ],
            ..EmitterLook::default()
        }
    }
}

/// Grid pitch of the dot header, in logical pixels.
const DOT_PITCH: f32 = 2.2;
/// Columns between the word count and the clock.
const DOT_GAP: i32 = 14;
/// Ids per dot text in the shared field.
const DOT_SPAN: u32 = 4096;
/// Size of the dot check in cells.
const CHECK_CELLS: (i32, i32) = (9, 8);

/// The header as one dot field across its whole width: the clock in the
/// large dot font and the word count in the small one at the right, and on
/// success a check at the left, assembled from dots that leave the clock.
/// Changed digits morph: their dots travel to the new digit.
struct DotHeader {
    field: DotField,
    clock: DotText,
    count: DotText,
    /// The field as the element's source; replaced on every change, since
    /// the element compares sources by identity.
    source: Arc<DotField>,
    slate: bool,
    checked: bool,
    /// Size of the element in cells.
    cols: i32,
    rows: i32,
}

impl DotHeader {
    fn new(t: f64, cols: i32) -> DotHeader {
        let (large, small) = (DotFont::LARGE, DotFont::SMALL);
        let clock_x = cols - large.width("00:00", 1);
        let count_x = clock_x - DOT_GAP - small.width(&count_text(0), 1);
        let field = DotField::new(t, Springs::default());
        let clock = DotText::new(
            DotStyle::new(large, c(ICE)),
            (clock_x as f32, 0.0),
            0,
            DOT_SPAN,
        );
        // Sit the small capitals on the clock's baseline.
        let drop = (large.ascent() - small.ascent()) as f32;
        let count = DotText::new(
            DotStyle::new(small, c(TEAL)),
            (count_x as f32, drop),
            DOT_SPAN,
            DOT_SPAN,
        );
        DotHeader {
            source: Arc::new(field.clone()),
            field,
            clock,
            count,
            slate: false,
            checked: false,
            cols,
            rows: large.line_height(),
        }
    }

    /// Forms a check in the icon slot from dots that gather in from a
    /// little way round it, so nothing crosses the title or the clock.
    fn check(&mut self, t: f64) {
        let cells = check_cells(CHECK_CELLS.0, CHECK_CELLS.1, 2);
        let top = ((self.rows - CHECK_CELLS.1) / 2) as f32;
        let (cx, cy) = (CHECK_CELLS.0 as f32 / 2.0, top + CHECK_CELLS.1 as f32 / 2.0);
        let from: Vec<DotLook> = cells
            .iter()
            .map(|&(x, y)| {
                let (dx, dy) = (x as f32 - cx, y as f32 + top - cy);
                let mut l = DotLook::new(cx + 2.5 * dx, cy + 2.5 * dy, 0.5, c(EMBER));
                l.bright = 0.0;
                l
            })
            .collect();
        let look = DotLook::new(0.0, 0.0, 0.75, c(EMBER));
        assemble(
            &mut self.field,
            t,
            &from,
            &cells,
            (0.0, top),
            look,
            2 * DOT_SPAN,
            0.02,
            0.25,
        );
    }
}

/// "  8 WORDS": fixed width up to 999 words, so the digits stay in place.
fn count_text(words: usize) -> String {
    format!("{:>3} WORDS", words.min(999))
}

impl Extra {
    /// Moves the dot header to the current clock and word count, and
    /// assembles the check as soon as the final text arrives, so it lands
    /// before the popup closes.
    fn sync_dots(&mut self, t: f64, cols: i32, clock: &str, words: usize, phase: Phase) {
        let final_seen = self.final_at.is_some();
        let h = self.dots.get_or_insert_with(|| DotHeader::new(t, cols));
        let before = (h.clock.text(), h.count.text(), h.slate, h.checked);
        // Changed digits morph dot by dot; the rest stay put.
        h.clock.set(&mut h.field, t, clock, Change::Morph);
        h.count
            .set(&mut h.field, t, &count_text(words), Change::Morph);
        let finishing = phase == Phase::Finishing;
        if finishing != h.slate {
            h.slate = finishing;
            let hue = if finishing { SLATE } else { ICE };
            h.clock.recolor(&mut h.field, t, c(hue));
        }
        if final_seen && !h.checked {
            h.checked = true;
            h.check(t);
        }
        if before != (h.clock.text(), h.count.text(), h.slate, h.checked) {
            h.field.prune(t);
            h.source = Arc::new(h.field.clone());
        }
    }

    /// Notes the stop and the final text from a change of the runtime files.
    pub fn files(&mut self, old: &Files, new: &Files, t: f64) {
        let finishing = |f: &Files| StatusLine::new(&f.status, "").finishing;
        if finishing(new) && self.finish_at.is_none() {
            self.finish_at = Some(t);
        }
        // The final text replaces the live preview while the final pass runs.
        if self.finish_at.is_some() && new.preview != old.preview && !new.preview.is_empty() {
            self.final_at = Some(t);
        }
        super::type_look::note_icon(self, old, new, t);
    }
}

impl Popup {
    fn look_is_showcase(&self) -> bool {
        self.look.theme == OsdTheme::Showcase
    }

    /// Brings the showcase's own state up to date after a message.
    pub(crate) fn sync_showcase(&mut self, t: f64) {
        if !self.look_is_showcase() {
            return;
        }
        let phase = self.showcase_phase();
        let level = if phase.ambient() {
            self.level_now()
        } else {
            0.0
        };
        if self.extra.level_sent != Some(level) {
            self.extra.level.push(t, level);
            self.extra.level_sent = Some(level);
        }
        let cols = (self.look.text_width() / DOT_PITCH).floor() as i32;
        self.extra
            .sync_dots(t, cols, &self.clock, self.word_count, phase);
    }

    /// The dot-matrix header across the whole row, glowing a little.
    fn dot_header(&self) -> Option<Element<Msg>> {
        let h = self.extra.dots.as_ref()?;
        let dots = Dots::new(h.source.clone(), DOT_PITCH);
        let (w, hh) = (h.cols as f32 * DOT_PITCH, h.rows as f32 * DOT_PITCH);
        let y = (Self::header_h(&self.look) - hh) / 2.0;
        Some(
            at(Element::layer("dotlights"), 0.0, y)
                .size(w, hh)
                .bloom(0.45, 4.0)
                .child(Element::dots("dots", dots).size(w, hh)),
        )
    }

    /// Seconds the level has been quiet, from the newest levels.
    fn quiet_secs(&self) -> f64 {
        let quiet = self
            .levels
            .iter()
            .rev()
            .take_while(|l| (**l as f32) < sc::QUIET_LEVEL)
            .count();
        quiet as f64 / FRAME_HZ as f64
    }

    /// The level now: the loudest of the last few frames, 0 to 1.
    fn level_now(&self) -> f32 {
        if self.files.recording || !self.status().finishing {
            self.levels
                .iter()
                .rev()
                .take(6)
                .fold(0.0f64, |a, b| a.max(*b)) as f32
        } else {
            0.0
        }
    }

    pub(crate) fn showcase_phase(&self) -> Phase {
        let status = self.status();
        sc::phase(
            status.loading,
            status.finishing,
            self.extra.final_at.is_some(),
            self.quiet_secs(),
        )
    }

    /// Whether timed view updates should run at `t`: the open and the glow
    /// pulse after the final text. Ambient motion runs on canvases.
    pub(crate) fn showcase_ticks(&self, t: f64) -> bool {
        self.look_is_showcase()
            && !self.closing
            && (t < self.shown_at + sc::OPEN_SECS
                || self
                    .extra
                    .final_at
                    .is_some_and(|f| t < f + sc::FINAL_PULSE_SECS))
    }

    /// Keeps the ambient tick armed while it is needed.
    pub(crate) fn arm_tick(&mut self, cx: &mut Cx<Msg>) {
        if self.extra.tick.is_none() && self.showcase_ticks(cx.t()) {
            self.extra.tick = Some(cx.after(1.0 / sc::AMBIENT_HZ, Msg::Tick));
        }
    }

    /// The card's height with `lines` transcript lines.
    pub(crate) fn showcase_card_height(look: &Look, lines: u32) -> f32 {
        2.0 * BORDER
            + PAD_TOP
            + Self::header_h(look)
            + 2.0 * RIBBON_MARGIN
            + sc::RIBBON_PX
            + lines as f32 * look.text_line
            + 2.0 * STRIP_PAD_Y
            + STATUS_MARGIN_TOP
            + Self::status_h(look)
            + PAD_BOTTOM
    }

    fn header_h(look: &Look) -> f32 {
        let dots = DotFont::LARGE.line_height() as f32 * DOT_PITCH;
        look.mono_line(look.value_px * 1.25).max(ICON).max(dots)
    }

    fn status_h(look: &Look) -> f32 {
        look.mono_line(look.status_px).max(STATUS_DOT)
    }

    /// The surface: the card at its tallest plus the margin for the glow.
    pub(crate) fn showcase_surface(look: &Look) -> Size {
        let h = Self::showcase_card_height(look, swiss::MAX_LINES as u32);
        Size::new(
            (look.width + 2.0 * sc::MARGIN_PX).ceil(),
            (h + 2.0 * sc::MARGIN_PX).ceil(),
        )
    }

    pub(crate) fn showcase_view(&self) -> Element<Msg> {
        let l = &self.look;
        let m = sc::MARGIN_PX;
        let root = Element::column("root")
            .size(
                l.width + 2.0 * m,
                Self::showcase_card_height(l, 5) + 2.0 * m,
            )
            .pad(m);
        if self.closing {
            return root;
        }
        let phase = self.showcase_phase();
        let card_h = Self::showcase_card_height(l, self.lines);
        root.child(self.backdrop(phase, card_h))
            .child(self.card(phase))
            .child(self.front(phase, card_h))
    }

    /// The glow pulse after the final text, 1 at its start, 0 when done.
    fn final_pulse(&self) -> f32 {
        self.extra.final_at.map_or(0.0, |f| {
            let k = ((self.extra.now - f) / sc::FINAL_PULSE_SECS).clamp(0.0, 1.0) as f32;
            1.0 - k
        })
    }

    /// The card's glow: colour and strength. It breathes with the level
    /// while listening, sinks to slate while finishing and flares once when
    /// the final text lands.
    fn aura(&self, phase: Phase) -> (OsdColor, f32) {
        let pulse = self.final_pulse();
        match phase {
            Phase::Done => (mix(TEAL, ICE, pulse), 0.05 + 0.2 * pulse),
            Phase::Finishing => (SLATE, 0.04),
            _ if phase.ambient() => (TEAL, 0.05 + 0.1 * self.level_now()),
            _ => (TEAL, 0.04),
        }
    }

    /// Ring and orbs behind the card, added as light, with a bloom. Drawn
    /// from the frame time on a canvas, so they turn and drift smoothly at
    /// any refresh rate and stop where they are once the recording stops.
    fn backdrop(&self, phase: Phase, card_h: f32) -> Element<Msg> {
        let l = &self.look;
        let m = sc::MARGIN_PX;
        let level = self.level_now();
        let live = phase.ambient();
        let surface = Self::showcase_surface(l);
        let (rx, ry) = (l.width * 0.42, 26.0);
        let (cx0, cy0) = (m + l.width / 2.0, m - 2.0);
        // Out-of-focus orbs drifting in depth around the card.
        let orbs = OrbField {
            level: self.extra.level.clone(),
            half: [l.width / 2.0 + 0.3 * m, card_h / 2.0 + 0.2 * m],
            origin: Point::new(0.0, m + card_h / 2.0 - surface.h / 2.0),
            shown_at: self.shown_at,
            finish_at: self.extra.finish_at,
        };
        let mut orbs = Emitter::new(Arc::new(orbs), ORB_PARTICLES, 11);
        orbs.three_d = true;
        // The tilted ring around the top edge of the card, as particles. Its
        // back half shows above the card; the card hides the front half.
        let ring = VoiceRing {
            level: self.extra.level.clone(),
            origin: Point::new(cx0 - surface.w / 2.0, cy0 - surface.h / 2.0),
            radius: rx,
            tilt: (ry / rx).acos(),
            shown_at: self.shown_at,
            finish_at: self.extra.finish_at,
            final_at: self.extra.final_at,
        };
        at(Element::layer("back"), 0.0, 0.0)
            .size(surface.w, surface.h)
            .blend(BlendMode::Add)
            .bloom(if live { 0.2 + 0.2 * level } else { 0.1 }, 6.0)
            // Swirling particles leave short comet tails.
            .trail(0.06)
            .child(
                at(
                    Element::emitter("ring", Emitter::new(Arc::new(ring), RING_PARTICLES, 7)),
                    0.0,
                    0.0,
                )
                .size(surface.w, surface.h)
                .presence(Transition::fade(), Transition::fade()),
            )
            .child(
                at(Element::emitter("orbs", orbs), 0.0, 0.0)
                    .size(surface.w, surface.h)
                    .presence(Transition::fade(), Transition::fade().offset(0.0, -24.0)),
            )
    }

    /// The card: glass-dark body, header, ribbon, transcript strip, status.
    fn card(&self, phase: Phase) -> Element<Msg> {
        let l = &self.look;
        let aura = self.aura(phase);
        let mut tint = c(mix(INK_HIGH, INK, 0.5));
        tint.a = l.background.a.max(0.92);
        // Success leaves upward, cancel sinks away quietly.
        let exit = if self.came_to_nothing() {
            Transition::fade().scale(0.97)
        } else {
            Transition::fade().offset(0.0, -12.0)
        };
        Element::panel("card")
            .width(l.width)
            .pad_edges(
                PAD_TOP + BORDER,
                PAD_X + BORDER,
                PAD_BOTTOM + BORDER,
                PAD_X + BORDER,
            )
            .radius(sc::RADIUS_PX)
            .glass(self.card_glass(tint))
            // The rim element in front draws the border line.
            .border(Color::TRANSPARENT, BORDER)
            .glow(c(aura.0), GLOW_PX, aura.1)
            .motion(Prop::Glow, Spring::SOFT)
            .motion(Prop::Scale, Spring::POP)
            .presence(Transition::fade().scale(0.94).offset(0.0, 14.0), exit)
            .child(self.sc_header(phase))
            .child(self.ribbon(phase))
            .child(self.strip(phase))
            .child(self.sc_status(phase))
    }

    /// Dark smoked glass in the card's ink, with its sheen on show. The
    /// compositor blurs the desktop behind it where it can.
    fn card_glass(&self, tint: Color) -> Glass {
        let glass = Glass {
            accent: c(EMBER),
            shadow: [0.55, 18.0, 10.0],
            ..Glass::dark()
        };
        // The clearest tint that keeps the text and the signal colour at 4.5:1
        // over any desktop.
        glass
            .tinted_for(tint, &[c(ICE), c(TEAL)], 4.5)
            .unwrap_or(Glass { tint, ..glass })
    }

    fn sc_header(&self, phase: Phase) -> Element<Msg> {
        let l = &self.look;
        let big = l.value_px * 1.25;
        let icon = if self.extra.final_at.is_some() && self.extra.dots.is_some() {
            // The dot header draws the check here.
            Element::column("check").size(ICON, ICON)
        } else if phase == Phase::Done {
            // The check: two strokes in ember, when there are no dots.
            Element::column("check")
                .size(ICON, ICON)
                .child(
                    at(Element::shape("a", ShapeKind::Line), 3.0, 9.0)
                        .size(4.0, 4.0)
                        .stroke(c(EMBER), 2.2)
                        .glow(c(EMBER), 4.0, 0.5),
                )
                // A line runs from its box's top left to bottom right; a
                // quarter turn makes this one rise to the right.
                .child(
                    at(Element::shape("b", ShapeKind::Line), 7.0, 4.0)
                        .size(9.0, 9.0)
                        .rotate(-std::f32::consts::FRAC_PI_2)
                        .stroke(c(EMBER), 2.2)
                        .glow(c(EMBER), 4.0, 0.5),
                )
                .presence(Transition::fade().scale(0.3), Transition::fade())
                .motion(Prop::Scale, Spring::POP)
        } else {
            self.mic_icon(phase)
        };
        let title = Element::label("title", "VOICE DICTATION")
            .text_style(l.mono(l.caption_px).tracking(2.4))
            .color(c(ICE).fade(0.55))
            .margin_edges(0.0, 0.0, 0.0, 10.0);
        let words = format!("{} words", self.word_count);
        let row = Element::row("header")
            .height(Self::header_h(l))
            .align_items(AlignItems::CENTER)
            .child(icon)
            .child(title)
            .child(Element::row("spacer").grow(1.0));
        if let Some(dots) = self.dot_header() {
            return row.child(dots);
        }
        row.child(
            Element::label("words", words)
                .text_style(l.mono(l.caption_px).tracking(1.0))
                .color(c(TEAL))
                .margin_edges(0.0, 14.0, 0.0, 0.0),
        )
        .child(
            Element::label("clock", self.clock.as_str())
                .text_style(l.mono(big).tracking(1.5))
                .color(c(ICE)),
        )
    }

    /// The level as light: three waves of one history, a wide faint halo, the
    /// body and a bright core, added together.
    fn ribbon(&self, phase: Phase) -> Element<Msg> {
        let l = &self.look;
        let frozen = matches!(phase, Phase::Finishing | Phase::Done);
        let (body, core) = if frozen { (SLATE, SLATE) } else { (TEAL, ICE) };
        let wave = |key: &'static str, h: f32, hue: OsdColor, alpha: f32, min_half: f32| {
            let stops = vec![
                (0.0, c(hue).fade(0.0)),
                (0.25, c(hue).fade(alpha * 0.8)),
                (0.85, c(hue).fade(alpha)),
                (1.0, c(mix(hue, ICE, 0.4)).fade(alpha)),
            ];
            let w = Wave {
                levels: self.wave.clone(),
                newest_at: self.newest_at,
                rate: FRAME_HZ as f32,
                window: l.window as f32,
                stops,
                min_half,
                baseline: None,
            };
            let y = (sc::RIBBON_PX - h) / 2.0;
            at(Element::wave(key, w), 0.0, y).size(l.text_width(), h)
        };
        // The bloom spreads the light round the wave; no faked halo.
        let bloom = if frozen {
            0.2
        } else {
            0.3 + 0.4 * self.level_now()
        };
        let lights = Element::layer("lights")
            .size(l.text_width(), sc::RIBBON_PX)
            .blend(BlendMode::Add)
            .bloom(bloom, 8.0)
            .motion(Prop::Bloom, Spring::SOFT)
            .child(wave("body", sc::RIBBON_PX * 0.6, body, 0.45, 0.75))
            .child(wave("core", sc::RIBBON_PX * 0.22, core, 0.8, 0.5));
        Element::column("ribbon")
            .height(sc::RIBBON_PX)
            .margin_edges(RIBBON_MARGIN, 0.0, RIBBON_MARGIN, 0.0)
            .child(
                at(
                    Element::shape("base", ShapeKind::RRect),
                    0.0,
                    sc::RIBBON_PX / 2.0,
                )
                .size(l.text_width(), 1.0)
                .fill(c(SLATE).fade(0.35)),
            )
            .child(lights)
    }

    /// The calm zone: a matte strip with the live words, where nothing else
    /// moves.
    fn strip(&self, phase: Phase) -> Element<Msg> {
        let l = &self.look;
        // One brief brightening when the final text lands.
        let text = mix(ICE, OsdColor::rgb(1.0, 1.0, 1.0), self.final_pulse());
        let mut words = Element::live_text("words", &self.shown)
            .text_style(l.text_style())
            .color(c(text))
            .motion(Prop::TextColor, Spring::SOFT)
            // Fast enough to read in under 200 ms; words rise from the
            // ribbon above.
            .motion(Prop::Opacity, Spring::critical_to_98(0.10))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.15).offset(0.0, -10.0))
            .word_change(Transition::fade().opacity(0.3).offset(0.0, 5.0));
        if phase == Phase::Done {
            words = words.word_change(Transition::fade().opacity(0.5));
        }
        // While listening, each new word forms from particles; a revision
        // scatters and re-forms only the glyphs that changed. Not in the
        // theme wall: word ids repeat from cell to cell, and the wall keeps
        // word particles for the look built on them.
        if phase.ambient() && !self.in_wall {
            words = words.word_particles(WordParticles {
                color: c(mix(TEAL, ICE, 0.5)).fade(0.7),
                size: 0.6,
                density: 0.6,
                ..WordParticles::default()
            });
        }
        // A faint teal halo under the live words, drawn from the glyphs'
        // distance field; it widens and turns to ice once when the final
        // text lands.
        let pulse = self.final_pulse();
        let (halo, radius) = match phase {
            _ if self.extra.final_at.is_some() => {
                (c(ICE).fade(0.12 + 0.35 * pulse), 4.0 + 8.0 * pulse)
            }
            Phase::Finishing | Phase::Done => (c(SLATE).fade(0.12), 4.0),
            _ => (c(TEAL).fade(0.22), 4.0),
        };
        let words = words
            .text_halo(halo, radius)
            .motion(Prop::TextFx, Spring::SOFT);
        let mut strip_fill = c(MATTE);
        strip_fill.a = 0.9;
        Element::panel("strip")
            .height(self.lines as f32 * l.text_line + 2.0 * STRIP_PAD_Y)
            .pad_edges(STRIP_PAD_Y, STRIP_PAD_X, STRIP_PAD_Y, STRIP_PAD_X)
            .margin_edges(0.0, -STRIP_PAD_X, 0.0, -STRIP_PAD_X)
            .radius(8.0)
            .fill(strip_fill)
            .hairline(c(SLATE).fade(0.25))
            // A specular edge at the top left, and one sheen across the
            // strip when it appears.
            .rim(c(ICE).fade(0.35), 1.0, 0.12, -0.375, 0.0)
            .sheen(c(ICE).fade(0.10), 0.7)
            .child(words)
    }

    fn sc_status(&self, phase: Phase) -> Element<Msg> {
        let l = &self.look;
        let status = self.status();
        let (accent, filled) = match phase {
            Phase::Finishing => (EMBER, false),
            Phase::Done => (TEAL, true),
            Phase::Loading => (ICE, false),
            _ => (TEAL, true),
        };
        let dot = Element::shape("dot", ShapeKind::Circle).size(STATUS_DOT, STATUS_DOT);
        let dot = if filled {
            dot.fill(c(accent)).glow(c(accent), 6.0, 1.0)
        } else {
            dot.fill(Color::TRANSPARENT).stroke(c(accent), 1.4)
        };
        let label = match phase {
            Phase::Paused => "Listening".to_string(),
            Phase::Done => "Done".to_string(),
            _ => status.state.clone(),
        };
        let mut row = Element::row("status")
            .height(Self::status_h(l))
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_MARGIN_TOP, 0.0, 0.0, 0.0)
            .child(dot)
            .child(
                Element::label("state", label.to_uppercase())
                    .text_style(l.mono(l.status_px).tracking(1.8))
                    .color(c(accent))
                    .margin_edges(0.0, 0.0, 0.0, 10.0),
            )
            .child(
                Element::label(
                    "detail",
                    if phase == Phase::Done {
                        ""
                    } else {
                        status.detail.as_str()
                    },
                )
                .text_style(l.mono(l.status_px).clamp_lines(1))
                .color(c(SLATE))
                .shrink(1.0)
                .min_width(0.0),
            )
            .child(Element::row("spacer").grow(1.0));
        if phase.ambient() && !self.shows_progress(&status) {
            row = row.child(self.level_meter());
        }
        if self.shows_progress(&status) {
            match self.files.fraction {
                Some(f) => {
                    // A dot meter: lit dots for the fraction done.
                    let lit = lit_dots(f as f32, METER_DOTS);
                    let mut meter = Element::row("meter")
                        .gap(3.0)
                        .align_items(AlignItems::CENTER)
                        .margin_edges(0.0, 0.0, 0.0, 12.0);
                    for i in 0..METER_DOTS {
                        let on = i < lit;
                        meter = meter.child(
                            Element::shape(format!("m{i}"), ShapeKind::Circle)
                                .size(4.0, 4.0)
                                .fill(if on { c(TEAL) } else { c(SLATE).fade(0.35) })
                                .motion(Prop::Fill, Spring::SNAP),
                        );
                    }
                    row = row.child(meter);
                }
                None => {
                    let progress = Progress {
                        fraction: None,
                        track: c(SLATE).fade(0.35),
                        bar: c(TEAL),
                        since: self.shown_at,
                        period: 1.6,
                    };
                    row = row.child(
                        Element::progress("progress", progress)
                            .size(120.0, 2.0)
                            .margin_edges(0.0, 0.0, 0.0, 12.0),
                    );
                }
            }
        }
        row
    }

    /// The mic as a stroked vector icon that draws on with the card, then
    /// morphs into the stop square when the recording stops.
    fn mic_icon(&self, phase: Phase) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 48))
        });
        let (hue, glow) = if phase == Phase::Finishing {
            (SLATE, 0.0)
        } else {
            (EMBER, 1.5)
        };
        let style = StrokeStyle::solid(1.6, c(hue)).glow(glow);
        // The mic turns into the stop square when the recording stops.
        Element::morph("mic", morph.clone(), Rect::new(0.0, 0.0, 24.0, 24.0), style)
            .size(ICON, ICON)
            .morph_to(if self.extra.finish_at.is_some() {
                1.0
            } else {
                0.0
            })
            .motion(Prop::Morph, Spring::critical_to_98(MORPH_SECS))
            .presence(Transition::fade(), Transition::fade().scale(0.5))
    }

    /// A dot level meter with peak hold, teal into ice into ember.
    fn level_meter(&self) -> Element<Msg> {
        const LEN: u32 = 24;
        let recent = self.levels.len().saturating_sub(FRAME_HZ as usize * 2);
        let levels: Vec<f32> = self.levels.iter().skip(recent).map(|l| *l as f32).collect();
        let mut meter = DotMeter::new(&levels, FRAME_HZ as f32, self.newest_at, LEN);
        meter.stops = vec![
            (0.0, c(TEAL)),
            (0.7, c(ICE)),
            (0.9, c(EMBER)),
            (1.0, c(EMBER)),
        ];
        meter.unlit = c(SLATE).fade(0.3);
        let pitch = 5.0;
        Element::layer("meterlights")
            .size(LEN as f32 * pitch, pitch)
            .bloom(0.5, 4.0)
            .child(
                Element::dots("meter", Dots::new(Arc::new(meter), pitch))
                    .size(LEN as f32 * pitch, pitch),
            )
    }

    /// Rim light and corner flares in front of the card, added as light.
    fn front(&self, phase: Phase, card_h: f32) -> Element<Msg> {
        let l = &self.look;
        let m = sc::MARGIN_PX;
        let surface = Self::showcase_surface(l);
        let level = self.level_now();
        let mut layer = at(Element::layer("front"), 0.0, 0.0)
            .size(surface.w, surface.h)
            .blend(BlendMode::Add);

        // The rim: a stroke round the card that draws on from the top centre
        // when the popup opens, with a light head that runs round it while
        // listening. From the stop on the head is gone and the rim calms.
        let pulse = self.final_pulse();
        let line = match phase {
            Phase::Finishing => c(SLATE).fade(0.6),
            Phase::Done => c(mix(EMBER, ICE, pulse)).fade(0.55),
            _ => c(EMBER).fade(0.45),
        };
        let border = Arc::new(Outline {
            contours: vec![make::panel(m, m, l.width, card_h, sc::RADIUS_PX)],
        });
        let view = Rect::new(0.0, 0.0, surface.w, surface.h);
        let mut rim = at(
            Element::stroke_path("rim", border, view, StrokeStyle::solid(1.2, line)),
            0.0,
            0.0,
        )
        .size(surface.w, surface.h)
        .motion(Prop::Trim, Spring::critical_to_98(sc::OPEN_SECS))
        .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade());
        if phase.ambient() {
            let bright = if phase == Phase::Paused {
                0.45
            } else {
                0.7 + 0.3 * level
            };
            rim = rim.light_head(
                0.14,
                1.0 / sc::ORBIT_LAP_SECS as f32,
                c(mix(EMBER, ICE, 0.35)).fade(bright),
                4.0,
            );
        }
        layer = layer.child(rim);

        // Corner flares: short crossed lines that pulse with the level.
        if phase.ambient() {
            let flare = 0.15 + 0.6 * level;
            let len = 10.0 + 18.0 * level;
            let corners = [
                (m, m),
                (m + l.width, m),
                (m + l.width, m + card_h),
                (m, m + card_h),
            ];
            for (i, (x, y)) in corners.into_iter().enumerate() {
                for (j, (w, h)) in [(len, 0.0), (0.0, len)].into_iter().enumerate() {
                    layer = layer.child(
                        at(
                            Element::shape(format!("flare{i}{j}"), ShapeKind::Line),
                            x - w / 2.0,
                            y - h / 2.0,
                        )
                        .size(w.max(0.01), h.max(0.01))
                        .stroke(c(EMBER).fade(flare), 1.2)
                        .glow(c(EMBER), 3.0, 0.3 * flare)
                        .presence(Transition::fade(), Transition::fade()),
                    );
                }
            }
        }
        layer
    }
}
