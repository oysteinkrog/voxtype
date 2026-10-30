//! The `Default` and `Swiss` look of the vo popup: the panel with its header,
//! waveform, transcript, rule and status row. The other looks fall through to it
//! as well, and several reuse its pieces.
//!
//! While the swiss popup is open it keeps a little idle life (v41, RD-02):
//! a soft highlight runs along the top hairline once every 8 s, the
//! recording dot breathes from 100 % to 70 % while listening, and the
//! clock's colon dims to half for a quarter second as each second ticks. All three
//! are ambient loops stepped at 15 a second, far from the transcript. They
//! leave the view in the update that asks to exit and on a look switch, and
//! hold still under reduced motion.

use super::*;
use vo::scene::ambient::{Loop, LoopProp, Wave as LoopWave};
use vo::scene::display::{Paint, ShapeInstance};
use vo::scene::geom::{Point, RRect};

/// v41: the idle loops' pace, the slow-loop cap of RD-02.
const IDLE_FPS: f64 = 15.0;
/// v41: the shimmer's width, how often it runs, how long one pass takes,
/// when the first pass comes after the popup opens (all seconds but the
/// width), and how much lighter than the hairline it is (HSL lightness).
const SHIMMER_W: f32 = 60.0;
const SHIMMER_EVERY: f64 = 8.0;
const SHIMMER_PASS: f64 = 1.2;
const SHIMMER_FIRST: f64 = 2.0;
const SHIMMER_LIGHTER: f32 = 0.2;
/// v41: the recording dot's lowest opacity and seconds per breath.
const DOT_LOW: f64 = 0.7;
const DOT_PERIOD: f64 = 2.4;
/// v41: the clock colon's opacity while soft and how long it stays soft
/// at each whole second.
const COLON_SOFT: f64 = 0.5;
const COLON_SECS: f64 = 0.25;

/// `c` with its HSL lightness raised by `amount`, hue and saturation kept.
fn lighter(c: Color, amount: f32) -> Color {
    let (r, g, b) = (c.r, c.g, c.b);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let l2 = (l + amount).clamp(0.0, 1.0);
    if d <= f32::EPSILON {
        return Color::rgba(l2, l2, l2, c.a);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    let q = if l2 < 0.5 {
        l2 * (1.0 + s)
    } else {
        l2 + s - l2 * s
    };
    let p = 2.0 * l2 - q;
    let ch = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    Color::rgba(ch(h + 1.0 / 3.0), ch(h), ch(h - 1.0 / 3.0), c.a)
}

/// v41: the shimmer's loop for a hairline `span` wide: the highlight
/// starts just left of the span and crosses it in `SHIMMER_PASS`, then
/// waits out of sight for the rest of `SHIMMER_EVERY`.
fn shimmer_loop(span: f32) -> Loop {
    let travel = (span + SHIMMER_W) as f64;
    Loop::new(
        LoopWave::Ramp,
        travel * SHIMMER_EVERY / SHIMMER_PASS,
        SHIMMER_EVERY,
    )
    .phase(1.0 - SHIMMER_FIRST / SHIMMER_EVERY)
    .fps(IDLE_FPS)
    .settle(Spring::SNAP)
}

/// v41: the dot's breath on opacity, 1.0 down to `DOT_LOW` and back.
fn dot_loop() -> Loop {
    Loop::pulse(DOT_LOW - 1.0, DOT_PERIOD)
        .fps(IDLE_FPS)
        .settle(Spring::SNAP)
}

/// v41: the colon's mask on opacity: `COLON_SOFT` for the first
/// `COLON_SECS` after the loop starts, then 0 for good. The mask is a new
/// node for each clock value, so the loop starts in the frame whose digits
/// change and the dim runs in step with the clock's own seconds.
fn colon_loop() -> Loop {
    // One cycle is far longer than any one clock value stays on screen.
    const ONCE: f64 = 3600.0;
    Loop {
        amp: COLON_SOFT,
        ..Loop::gate(0.0, COLON_SECS / ONCE, ONCE)
            .fps(IDLE_FPS)
            .settle(Spring::SNAP)
    }
}

impl Popup {
    /// Whether there is text above the first line shown, for the margin
    /// "…": earlier lines cut, or while editing, buffer lines above the
    /// window (plan section 2.4).
    pub(super) fn text_above(&self) -> bool {
        match &self.edit_view {
            Some(v) => v.start > 0,
            None => self.cut > 0,
        }
    }

    pub(super) fn panel(&self) -> Element<Msg> {
        let l = &self.look;
        let status = self.status();
        Element::panel("panel")
            .width(l.width)
            .pad_edges(
                PAD_TOP + BORDER,
                PAD_X + BORDER,
                PAD_BOTTOM + BORDER,
                PAD_X + BORDER,
            )
            .radius(RADIUS)
            .fill(l.background)
            .border(l.border, self.exhale_width(BORDER))
            .motion(Prop::StrokeWidth, self.exhale_snap())
            .child(self.header())
            .child(self.waveform(&status))
            .child(self.transcript())
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .height(1.0)
                    .fill(l.border)
                    .margin_edges(RULE_MARGIN_TOP, 0.0, 0.0, 0.0),
            )
            .child(self.status_row(&status))
            .children(self.swiss_shimmer())
    }

    /// v41: whether the swiss idle loops run: the swiss look, open, and
    /// motion not reduced.
    fn swiss_idle(&self) -> bool {
        matches!(self.look.theme, OsdTheme::Swiss | OsdTheme::Default)
            && !self.closing
            && !kit::reduced()
            && !self.look.on("reduced-motion")
    }

    /// v41: the highlight on the top hairline, in a layer clipped to the
    /// hairline's straight run between the rounded corners.
    fn swiss_shimmer(&self) -> Option<Element<Msg>> {
        if !self.swiss_idle() {
            return None;
        }
        let l = &self.look;
        let span = l.width - 2.0 * RADIUS;
        let hi = lighter(l.border, SHIMMER_LIGHTER);
        let clear = Color::rgba(hi.r, hi.g, hi.b, 0.0);
        let glint = Element::canvas("glint", 0.0, move |_, b, p| {
            let mid = Point::new(b.x + b.w / 2.0, b.y);
            p.shape(ShapeInstance::rrect(
                RRect::new(b, 0.0),
                Paint::sheen(mid, Point::new(b.w / 2.0, 0.0), hi, clear),
            ));
        });
        let glint = kit::at(glint, -SHIMMER_W, 0.0)
            .size(SHIMMER_W, BORDER)
            .ambient(LoopProp::OffsetX, shimmer_loop(span));
        Some(
            kit::at(Element::layer("shimmer").clip(), RADIUS, 0.0)
                .size(span, BORDER)
                .child(glint),
        )
    }

    /// v41: a patch of panel colour over the clock's colon whose opacity
    /// steps to `COLON_SOFT` as the clock's digits change, so the colon
    /// softens on each of the clock's seconds. Its key carries the clock
    /// value, so each new value brings a new mask and a fresh loop.
    fn swiss_colon(&self, value: &str, style: &TextStyle) -> Option<Element<Msg>> {
        let i = value.find(':')?;
        // The clock stops once the recording does; so does its colon.
        if !self.swiss_idle() || self.status().finishing {
            return None;
        }
        let before = self.measure.layout(&value[..i], style, None).size.w;
        let colon = self.measure.layout(":", style, None).size;
        let mask = Element::shape(format!("colon-{value}"), ShapeKind::RRect)
            .size(colon.w, colon.h)
            .fill(self.look.background)
            .opacity(0.0)
            .ambient(LoopProp::Opacity, colon_loop());
        Some(kit::at(mask, before, 0.0))
    }

    pub(super) fn header(&self) -> Element<Msg> {
        let l = &self.look;
        let caption = |key: &'static str, text: &str, px: f32, tracking: f32| {
            Element::label(key, text)
                .text_style(l.mono(px).tracking(tracking))
                .color(l.inactive)
        };
        let stat = |key: &'static str, value: &str, name: &'static str| {
            Element::column(key)
                .gap(2.0)
                .child(
                    self.exhale_motion(
                        Element::label("value", value)
                            .text_style(l.mono(l.value_px))
                            .color(if key == "time" {
                                self.exhale_color(l.foreground, l.inactive)
                            } else {
                                l.foreground
                            }),
                        &[Prop::TextColor],
                    ),
                )
                .child(caption("caption", name, l.caption_small_px, 1.5))
                .children(if key == "time" {
                    self.swiss_colon(value, &l.mono(l.value_px))
                } else {
                    None
                })
        };
        // The icon theme's microphone, or one from shapes: the capsule and its stand.
        let mic = match l.mic {
            Some((id, view)) => Element::path("mic", id, view)
                .size(ICON, ICON)
                .fill(l.accent),
            None => self.shape_mic(),
        };
        Element::row("header")
            .align_items(AlignItems::START)
            .child(mic)
            .child(
                caption("title", "VOICE DICTATION", l.caption_px, 2.0)
                    .margin_edges(0.0, 0.0, 0.0, 8.0),
            )
            .child(Element::row("spacer").grow(1.0))
            .child(stat("time", &self.clock, "DURATION"))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .width(1.0)
                    .fill(l.border)
                    .margin_edges(0.0, 16.0, 0.0, 16.0)
                    .layout(|s| s.align_self = Some(AlignSelf::STRETCH)),
            )
            .child(stat("words", &self.word_count.to_string(), "WORDS"))
    }

    /// A microphone from shapes, for when the icon theme has none.
    pub(super) fn shape_mic(&self) -> Element<Msg> {
        let l = &self.look;
        Element::column("mic")
            .size(ICON, ICON)
            .align_items(AlignItems::CENTER)
            .gap(1.5)
            .child(
                Element::shape("body", ShapeKind::Capsule)
                    .size(6.0, 9.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("stand", ShapeKind::RRect)
                    .size(1.5, 2.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("base", ShapeKind::RRect)
                    .size(7.0, 1.5)
                    .fill(l.accent),
            )
    }

    pub(super) fn waveform(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        // Once the recording stops the frozen waveform turns grey.
        let (left, center, right) = if status.finishing {
            (l.inactive, l.inactive, l.inactive)
        } else {
            l.hues
        };
        let to_vo = |c: Color| OsdColor::rgba(c.r, c.g, c.b, c.a);
        let stops = wave_stops(to_vo(left), to_vo(center), to_vo(right))
            .into_iter()
            .map(|(p, c, a)| (p, Color::rgba(c.r, c.g, c.b, a)))
            .collect();
        let wave = Wave {
            levels: self.exhale_levels(&self.wave),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
            window: l.window as f32,
            stops,
            min_half: 0.5,
            baseline: Some(l.border),
        };
        Element::wave("wave", wave)
            .height(swiss::WAVE_HEIGHT_PX as f32)
            .margin_edges(WAVE_MARGIN, 0.0, WAVE_MARGIN, 0.0)
    }

    pub(super) fn transcript(&self) -> Element<Msg> {
        let l = &self.look;
        let mut words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(l.foreground);
        // Without word_fade nothing moves: a new word is on screen at full
        // opacity in the first frame after it arrives, as in GTK.
        if l.word_fade {
            words = words
                .motion(Prop::Opacity, Spring::SNAP)
                .motion(Prop::Offset, Spring::SNAP)
                .word_enter(self.ghost_enter(Transition::fade().offset(0.0, 3.0)));
        }
        let words = self.typo_words(words);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            l.foreground,
            l.accent,
        );
        let text = Element::column("text")
            .height(self.lines as f32 * l.text_line)
            .children(self.swiss_lattice())
            .child(words)
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink::of(l),
                endings::Cancel::Unthread,
            ));
        let text = self.typo_marks(text, l.text_width());
        let text = self.signal_marks(text, &self.shown, l.text_width(), (0.0, 0.0), l.foreground);
        let text = self.with_hearing(
            text,
            &l.text_style(),
            l.text_width(),
            l.foreground,
            l.accent,
        );
        self.text_area(text)
    }

    /// LT-07: with `kinetic-lattice` on the swiss look, the lattice in the
    /// hairline style under the words. Its newest gutter reaches as far
    /// under the text as the text area's clip does.
    fn swiss_lattice(&self) -> Option<Element<Msg>> {
        let l = &self.look;
        if !matches!(l.theme, OsdTheme::Swiss | OsdTheme::Default)
            || !l.on(wild_space::KINETIC_LATTICE)
        {
            return None;
        }
        let set = wild_space::LatticeSet {
            preset: vo::scene::lattice::LatticeStyle::Hairline,
            color: l.inactive.fade(0.7),
            accent: l.accent,
            scale: 0.7,
            label: None,
            gutter: TEXT_MARGIN_BOTTOM + STATUS_MARGIN_TOP - 4.0 - 1.0,
            opacity: 1.0,
        };
        let style = l.text_style();
        self.lattice_lines("swiss", &self.shown, &style, l.text_width(), &set)
            .map(|e| kit::at(e, 0.0, 0.0))
    }

    /// The text area. For the looks whose [`Look::margin_ellipsis`] is on,
    /// it sits in a layer clipped to its own box, widened on the left for the
    /// "…" that hangs there once earlier lines are cut, so a line that leaves
    /// while the text scrolls slides behind the edge instead of crossing the
    /// wave or the status row.
    pub(super) fn text_area(&self, text: Element<Msg>) -> Element<Msg> {
        use vo::scene::taffy::{LengthPercentage, LengthPercentageAuto};
        let l = &self.look;
        if !l.margin_ellipsis() {
            return text.margin_edges(0.0, 0.0, TEXT_MARGIN_BOTTOM, 0.0);
        }
        let style = l.text_style();
        let w = self.measure.layout("…", &style, None).size.w.ceil();
        let mut text = text;
        if self.text_above() && !self.shown.is_empty() && self.delivered().is_none() {
            text = text.child(
                Element::label("ellipsis", "…")
                    .text_style(style)
                    .color(l.inactive)
                    .layout(|s| {
                        s.position = vo::scene::taffy::Position::Absolute;
                        s.inset = vo::scene::taffy::Rect {
                            left: LengthPercentageAuto::length(-w),
                            top: LengthPercentageAuto::length(0.0),
                            right: LengthPercentageAuto::auto(),
                            bottom: LengthPercentageAuto::auto(),
                        };
                    }),
            );
        }
        // The clip reaches into the gap above the status row, so descenders
        // and rising words on the last line are not cut; the negative margin
        // keeps the layout as it was.
        let below = TEXT_MARGIN_BOTTOM + STATUS_MARGIN_TOP - 4.0;
        // It reaches above the text only as far as a runner-up of
        // `twin-spelling` needs (0 when that is off).
        let above = self.twin_headroom();
        Element::layer("textclip")
            .clip()
            .margin_edges(-above, 0.0, TEXT_MARGIN_BOTTOM - below, -w)
            .layout(|s| {
                s.padding.top = LengthPercentage::length(above);
                s.padding.left = LengthPercentage::length(w);
                s.padding.bottom = LengthPercentage::length(below);
            })
            .child(text)
    }

    pub(super) fn status_row(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        // A filled square while listening, a hollow one while finishing.
        let dot = if status.finishing {
            Element::shape("dot", ShapeKind::RRect)
                .size(DOT, DOT)
                .fill(Color::TRANSPARENT)
                .border(l.accent, 1.0)
        } else {
            let dot = Element::shape("dot", ShapeKind::RRect)
                .size(DOT, DOT)
                .fill(l.accent);
            if self.swiss_idle() {
                dot.ambient(LoopProp::Opacity, dot_loop())
            } else {
                dot
            }
        };
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_MARGIN_TOP, 0.0, 0.0, 0.0)
            .child(dot)
            .child(
                Element::label("state", status.state.as_str())
                    .text_style(l.mono(l.status_px))
                    .color(l.accent)
                    .margin_edges(0.0, 0.0, 0.0, 10.0),
            )
            .child(
                Element::label("detail", status.detail.as_str())
                    .text_style(l.mono(l.status_px).clamp_lines(1))
                    .color(l.inactive)
                    .shrink(1.0)
                    .min_width(0.0),
            );
        if let Some(ruler) = self.audio_ruler(l.accent, l.border) {
            row = row.child(ruler.margin_edges(0.0, 0.0, 0.0, 16.0));
        } else if self.shows_progress(status) {
            let progress = Progress {
                fraction: self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32),
                track: l.border,
                bar: l.accent,
                since: self.shown_at,
                period: 1.6,
            };
            row = row.child(
                Element::progress("progress", progress)
                    .height(2.0)
                    .grow(1.0)
                    .margin_edges(0.0, 0.0, 0.0, 16.0),
            );
        }
        row
    }
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    #[test]
    fn lighter_raises_hsl_lightness_and_keeps_alpha() {
        let g = lighter(Color::rgba(0.25, 0.25, 0.25, 0.5), 0.2);
        assert!(
            (g.r - 0.45).abs() < 1e-5 && (g.g - 0.45).abs() < 1e-5 && (g.b - 0.45).abs() < 1e-5
        );
        assert_eq!(g.a, 0.5);
        let warm = Color::rgba(0.3, 0.25, 0.2, 1.0);
        let up = lighter(warm, 0.2);
        assert!(up.r > warm.r && up.g > warm.g && up.b > warm.b);
        assert!(up.r > up.g && up.g > up.b, "the hue stays: {up:?}");
        assert_eq!(lighter(Color::WHITE, 0.2).r, 1.0);
    }

    #[test]
    fn the_shimmer_crosses_once_every_8_s_and_hides_in_between() {
        let span = 540.0;
        let l = shimmer_loop(span);
        // The glint's left edge on the span, from its rest at -SHIMMER_W.
        let x = |t: f64| -SHIMMER_W as f64 + l.value(0.0, t);
        let shows = |t: f64| x(t) > -SHIMMER_W as f64 && x(t) < span as f64;
        for k in 0..3 {
            let t0 = SHIMMER_FIRST + k as f64 * SHIMMER_EVERY;
            assert!(shows(t0 + SHIMMER_PASS / 2.0), "mid pass {k}");
            assert!(!shows(t0 - 0.1), "before pass {k}");
            assert!(!shows(t0 + SHIMMER_PASS + 0.1), "after pass {k}");
        }
        assert!(!shows(0.0), "no pass as the popup opens");
        assert!(l.fps <= IDLE_FPS);
    }

    #[test]
    fn the_dot_breathes_between_70_and_100_percent() {
        let l = dot_loop();
        let mut lo: f64 = 0.0;
        for i in 0..100 {
            let v = l.value(0.0, i as f64 * 0.05);
            assert!((DOT_LOW - 1.0 - 1e-9..=1e-9).contains(&v), "{v}");
            lo = lo.min(v);
        }
        assert!((lo - (DOT_LOW - 1.0)).abs() < 0.01);
        assert!(l.fps <= IDLE_FPS);
    }

    #[test]
    fn the_colon_softens_once_for_a_quarter_second_from_its_start() {
        // The loop starts when the clock's digits change (a new mask node).
        let l = colon_loop();
        assert_eq!(
            l.value(0.0, 0.0),
            COLON_SOFT,
            "soft in the digits' first frame"
        );
        assert_eq!(l.value(0.0, 0.2), COLON_SOFT);
        assert_eq!(l.value(0.0, COLON_SECS + 0.07), 0.0);
        // A clock that stalls does not blink again on its own.
        for k in 1..10 {
            assert_eq!(l.value(0.0, k as f64), 0.0);
        }
        assert!(l.fps <= IDLE_FPS);
    }
}

/// The edit checks every look's edit bead runs (plan section 6, M7.L): a
/// dictation long enough to scroll, an edit of it with the caret in the
/// newest, a middle and the top window line, at the default popup width and
/// at a wide one (600 px, 17 pt). The caret must stand on the drawn glyphs,
/// within 1 px, in every frame of a reflow, and the popup keeps its size.
#[cfg(test)]
pub(crate) mod edit_check {
    use super::super::edit_link::{EditLink, Paths};
    use super::super::*;
    use vo::scene::display::{DisplayList, ItemKind};
    use vo::scene::geom::{Point, Rect, Size};
    use vo::scene::scene::Scene;
    use vo::scene::text::ShapedLayout;
    use vo_core::edit::EditAction;
    use voxtype::osd::edit_ipc::EditBlock;

    /// Which window line the caret goes to.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) enum CaretAt {
        Newest,
        Middle,
        Top,
    }

    pub(crate) struct Rig {
        pub p: Popup,
        pub scene: Scene<Msg>,
        tokens: vo::Tokens,
        _dir: tempfile::TempDir,
        pub t: f64,
    }

    /// The dictation: `n` short words. 160 are more lines than any look
    /// shows.
    pub(crate) fn dictation(n: usize) -> String {
        (0..n)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// One font registry for every edit check. A look keeps the ids of the
    /// fonts it adds in a process-wide `OnceLock`, taken from the first
    /// registry it sees, so tests that each load their own registry and run
    /// in parallel draw with each other's ids.
    pub(crate) fn fonts() -> FontRegistry {
        static FONTS: std::sync::OnceLock<FontRegistry> = std::sync::OnceLock::new();
        FONTS
            .get_or_init(|| load_fonts(&PlasmaTheme::default()).unwrap())
            .clone()
    }

    fn block(phase: &str, text: &str) -> EditBlock {
        EditBlock {
            session: 1,
            phase: phase.into(),
            frozen: text.into(),
            tail: String::new(),
            pause: 1,
            raw_words: text.split_whitespace().count(),
        }
    }

    impl Rig {
        /// `theme` at the default width, or `wide`: 600 px and 17 pt text.
        pub(crate) fn new(theme: OsdTheme, wide: bool) -> Rig {
            let mut cfg = OsdConfig {
                theme,
                ..OsdConfig::default()
            };
            if wide {
                cfg.width_px = 600;
                cfg.preview_font_pt = Some(17.0);
            }
            let plasma = PlasmaTheme::default();
            let fonts = fonts();
            let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
            look.set_motion(&cfg.motion);
            let dir = tempfile::TempDir::new().unwrap();
            let mut p = Popup::new(look, &fonts);
            p.edit = Some(EditLink::new(Paths {
                buffer: dir.path().join("edit-buffer.json"),
                cmds: dir.path().join("edit-cmd"),
            }));
            let mut scene: Scene<Msg> =
                Scene::new().with_text(Box::new(ShapedLayout::new(fonts.clone())));
            scene.set_viewport(0.0, p.look.live_surface(), 1.0);
            Rig {
                p,
                scene,
                tokens: vo::Tokens::default(),
                _dir: dir,
                t: 0.0,
            }
        }

        /// Shows the view at `t`.
        pub(crate) fn show(&mut self, t: f64) {
            self.t = t;
            let view = self.p.view(&self.tokens);
            self.scene.apply(view, t);
        }

        /// The popup's size: its surface and the laid-out root.
        pub(crate) fn size(&self) -> (Size, Rect) {
            let root = self.scene.root().expect("a view");
            (self.p.look.live_surface(), self.scene.layout_target(root))
        }

        /// The dictation of `n` words shown at 0.
        pub(crate) fn dictate(&mut self, n: usize) {
            let text = dictation(n);
            self.p.files.preview = text.clone();
            self.p.text(&text);
            self.show(0.0);
        }

        /// A tap at `t`: the popup owns the text, the caret at its end.
        pub(crate) fn enter(&mut self, t: f64) {
            let text = self.p.files.preview.clone();
            let link = self.p.edit.as_mut().unwrap();
            link.sync(Some(&block("editing", &text)));
            self.p.sync_quiet();
            assert!(self.p.edit_text(t));
            self.show(t);
        }

        /// Resume: the daemon listens again and the popup shows the text.
        pub(crate) fn resume(&mut self, t: f64) {
            let text = self.buffer_text();
            let link = self.p.edit.as_mut().unwrap();
            link.resume();
            link.sync(Some(&block("listening", &text)));
            self.p.sync_quiet();
            assert!(!self.p.edit_text(t));
            self.p.files.preview = text.clone();
            let source = self.p.source_text();
            self.p.text(&source);
            self.show(t);
        }

        pub(crate) fn buffer_text(&self) -> String {
            let link = self.p.edit.as_ref().unwrap();
            link.buffer().unwrap().text().to_string()
        }

        /// One edit action at `t`, then the view.
        pub(crate) fn act(&mut self, action: EditAction, t: f64) {
            let site = edit_site::site(self.p.look.theme);
            let tb = site.text_box(&self.p.look);
            let text = self.buffer_text();
            let lines = self
                .p
                .edit_layer
                .shaped(&text, &self.p.measure, tb.style, tb.width)
                .clone();
            self.p.edit.as_mut().unwrap().apply(action, &lines, t);
            assert!(self.p.edit_text(t));
            self.show(t);
        }

        /// Moves the caret up from the newest window line to `at`, one line
        /// a key, starting at `t`. Returns the time after the last key.
        pub(crate) fn caret_to(&mut self, at: CaretAt, t: f64) -> f64 {
            let n = self.p.edit_view.as_ref().unwrap().lines;
            let up = match at {
                CaretAt::Newest => 0,
                CaretAt::Middle => n / 2,
                CaretAt::Top => n - 1,
            };
            let mut t = t;
            for _ in 0..up {
                let up = EditAction::MoveLine {
                    down: false,
                    extend: false,
                };
                self.act(up, t);
                t += 0.1;
            }
            let v = self.p.edit_view.as_ref().unwrap();
            assert_eq!(v.caret_line, n - 1 - up, "{at:?} of {n} lines");
            t
        }

        /// Every glyph drawn, `(word id, pen position)`, in surface space.
        pub(crate) fn glyphs(dl: &DisplayList) -> Vec<(u32, Point)> {
            let mut out = Vec::new();
            for (i, l) in dl.layers.iter().enumerate() {
                for it in dl
                    .layer_items(i as _)
                    .iter()
                    .filter(|it| it.kind == ItemKind::Glyphs)
                {
                    for g in &dl.glyphs[it.start as usize..(it.start + it.len) as usize] {
                        let q = l.transform.apply(g.pos);
                        out.push((g.word, l.warp.as_ref().map_or(q, |w| w.apply(q))));
                    }
                }
            }
            out
        }

        /// The glyph the caret follows in frame `dl`, its word's id, and the
        /// pens the caret must lie between: in a word, the glyph before the
        /// caret, and the next pen on the line; just before a word, that
        /// word's first glyph, and the pen of the glyph before it on the
        /// line. Just before a word that starts a wrapped line, the caret may
        /// stay at the end of the line above: then it follows the last glyph
        /// of the word before.
        pub(crate) fn caret_glyph(&self, dl: &DisplayList) -> Option<(u32, Point, f32, f32)> {
            let v = self.p.edit_view.as_ref()?;
            let caret = v.decor.caret?;
            let foot = dl.edit_caret?;
            let id = |w: usize| self.p.shown.get(w).map(|w| w.id);
            let ranges = edit::word_ranges(&v.text);
            let all = Self::glyphs(dl);
            let pens = |w: usize| {
                let mut own: Vec<Point> = all
                    .iter()
                    .filter(|g| Some(g.0) == id(w))
                    .map(|g| g.1)
                    .collect();
                own.sort_by(|a, b| a.x.total_cmp(&b.x));
                // A revised word's old glyphs fade where its new ones are.
                own.dedup_by(|a, b| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5);
                own
            };
            let on_line =
                |w: usize, y: f32| pens(w).into_iter().filter(move |p| (p.y - y).abs() < 1.0);
            if let Some(w) = ranges
                .iter()
                .position(|r| r.start < caret && caret <= r.end)
            {
                let own = pens(w);
                let k = caret - ranges[w].start - 1;
                let pen = *own.get(k)?;
                let hi = own
                    .get(k + 1)
                    .map(|p| p.x)
                    .or_else(|| on_line(w + 1, pen.y).map(|p| p.x).reduce(f32::min));
                return Some((id(w)?, pen, pen.x, hi.unwrap_or(f32::INFINITY)));
            }
            let w = ranges.iter().position(|r| r.start == caret)?;
            let pen = *pens(w).first()?;
            if (foot.y - pen.y).abs() > 1.0 && w > 0 {
                let last = *pens(w - 1).last()?;
                return Some((id(w - 1)?, last, last.x, f32::INFINITY));
            }
            let lo = w
                .checked_sub(1)
                .and_then(|b| on_line(b, pen.y).map(|p| p.x).reduce(f32::max));
            Some((id(w)?, pen, lo.unwrap_or(f32::NEG_INFINITY), pen.x))
        }

        /// Checks every frame from `from` to `to` at 120 fps: the caret is
        /// drawn, after its glyph and no further than the next on the line, on the
        /// glyph's baseline within 1 px, and in every frame within 1 px of
        /// the place against that glyph where it settles.
        pub(crate) fn caret_follows(&self, from: f64, to: f64, what: &str) {
            let settled = self.scene.sample(to);
            let (word, pen, lo, hi) = self.caret_glyph(&settled).expect("the caret's glyph");
            let foot = settled.edit_caret.expect("the caret");
            // Between two letters of a word the caret stands at the next
            // letter's pen.
            assert!(
                foot.x > lo && foot.x <= hi + 0.01,
                "{what}: caret {foot:?} between pens {lo} and {hi}"
            );
            assert!(
                (foot.y - pen.y).abs() <= 1.0,
                "{what}: caret {foot:?} on {pen:?}"
            );
            let rest = (foot.x - pen.x, foot.y - pen.y);
            let mut t = from;
            while t < to {
                let dl = self.scene.sample(t);
                let foot = dl.edit_caret.expect("the caret in every frame");
                // Some glyph of the caret's word is where the caret's glyph
                // must be. Its old copies fade out elsewhere, so the glyph is
                // not told apart by its place in the word.
                let off = Self::glyphs(&dl)
                    .into_iter()
                    .filter(|g| g.0 == word)
                    .map(|g| ((foot.x - g.1.x - rest.0).abs()).max((foot.y - g.1.y - rest.1).abs()))
                    .reduce(f32::min);
                // Typed letters are not drawn in the first frames of their
                // fade in (the still profile's 80 ms), and the caret may
                // follow one of them.
                let fading = t < from + 0.08;
                assert!(
                    fading || off.is_some_and(|d| d <= 1.0),
                    "{what}: at {t:.3} the caret is {off:?} px off its glyph"
                );
                t += 1.0 / 120.0;
            }
        }

        /// How many carets in `color` are drawn just after the last change,
        /// before any blink: upright bars at most 4 px wide.
        pub(crate) fn carets(&self, color: vo::scene::geom::Color) -> usize {
            let dl = self.scene.sample(self.t + 0.1);
            let near = |c: vo::scene::geom::Color| {
                c.a > 0.5
                    && (c.r - color.r).abs() < 0.02
                    && (c.g - color.g).abs() < 0.02
                    && (c.b - color.b).abs() < 0.02
            };
            dl.shapes
                .iter()
                .filter(|s| {
                    s.kind == vo::scene::display::ShapeKind::RRect
                        && s.geom[2] <= 4.0
                        && s.geom[3] >= 8.0
                        && near(s.fill.c0)
                })
                .count()
        }

        /// The golden's words for the view now: the window and where the
        /// caret settles.
        pub(crate) fn golden_part(&self) -> String {
            let v = self.p.edit_view.as_ref().unwrap();
            let foot = self.scene.sample(self.t + 2.0).edit_caret.unwrap();
            let words: Vec<&str> = v.text.split_whitespace().collect();
            format!(
                "{} lines, caret on line {} at {:.1},{:.1}, words {} to {}{}",
                v.lines,
                v.caret_line,
                foot.x,
                foot.y,
                words.first().copied().unwrap_or(""),
                words.last().copied().unwrap_or(""),
                if self.ellipsis_drawn() {
                    ", \u{2026}"
                } else {
                    ""
                },
            )
        }

        /// Whether a margin "…" is drawn: a glyph of no word left of every
        /// word's glyphs, level with the top line (a smaller "…" sits a
        /// little higher). Always false for a look with no margin "…",
        /// whose labels may sit there.
        pub(crate) fn ellipsis_drawn(&self) -> bool {
            if !self.p.look.margin_ellipsis() {
                return false;
            }
            let dl = self.scene.sample(self.t + 2.0);
            let glyphs = Self::glyphs(&dl);
            let words = glyphs.iter().filter(|g| g.0 != vo::scene::display::NO_WORD);
            let left = words.clone().map(|g| g.1.x).fold(f32::MAX, f32::min);
            let top = words.map(|g| g.1.y).fold(f32::MAX, f32::min);
            glyphs.iter().any(|g| {
                g.0 == vo::scene::display::NO_WORD && g.1.x < left && (g.1.y - top).abs() < 6.0
            })
        }

        /// The whole check for `theme`, compared with its golden,
        /// `tests/fixtures/osd-vo/edit-<name>.txt` (`VOXTYPE_BLESS=1` writes
        /// it).
        pub(crate) fn run(theme: OsdTheme, name: &str) {
            let mut golden = String::new();
            for wide in [false, true] {
                for at in [CaretAt::Newest, CaretAt::Middle, CaretAt::Top] {
                    let what = format!("{name} {} {at:?}", if wide { "wide" } else { "default" });
                    let mut r = Rig::new(theme, wide);
                    r.dictate(160);
                    let before = r.size();
                    r.enter(1.0);
                    assert_eq!(r.size(), before, "{what}: size at entry (R53)");
                    let t = r.caret_to(at, 2.0);
                    r.caret_follows(t, t + 0.5, &format!("{what}, moved"));
                    // The margin "…" shows while text is above the window.
                    if r.p.look.margin_ellipsis() {
                        assert!(r.ellipsis_drawn(), "{what}: the margin ellipsis");
                    }
                    let moved = r.golden_part();
                    // Typing reflows the caret's line: the words after the
                    // caret slide along and wrap, the caret with its glyph.
                    let t = t + 0.5;
                    r.act(EditAction::Insert("abcdefghij".into()), t);
                    r.caret_follows(t, t + 0.6, &format!("{what}, typed"));
                    assert_eq!(r.size(), before, "{what}: size while editing (R53)");
                    golden.push_str(&format!(
                        "{} {at:?}: {moved}; typed: {}\n",
                        if wide { "wide" } else { "default" },
                        r.golden_part()
                    ));
                    r.resume(t + 1.0);
                    assert_eq!(r.size(), before, "{what}: size after resume (R53)");
                }
            }
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/osd-vo/edit-{name}.txt"));
            if std::env::var_os("VOXTYPE_BLESS").is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &golden).unwrap();
            }
            let want = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                golden,
                want,
                "{} differs; VOXTYPE_BLESS=1 rewrites it",
                path.display()
            );
            // A text that fits: no "…" while editing it.
            for wide in [false, true] {
                let mut r = Rig::new(theme, wide);
                r.dictate(6);
                r.enter(1.0);
                assert!(!r.ellipsis_drawn(), "{name}: a short text has no ellipsis");
                assert_eq!(r.p.edit_view.as_ref().unwrap().start, 0);
            }
        }
    }

    /// R35: the edit moment (`moments::edit_scenario`) keeps the reading-line
    /// envelope, read on the caret line, with the caret on the newest line,
    /// one line up and on the top line (four Ups), at both widths. Checked
    /// from the moment the popup takes the keyboard until the edit ends: the
    /// dictation before it is the look's own `--envelope` run, and the
    /// delivery after it is the look's ending.
    pub(crate) fn envelope(theme: OsdTheme, name: &str) {
        let plasma = PlasmaTheme::default();
        let fonts = fonts();
        for wide in [false, true] {
            let mut cfg = OsdConfig {
                theme,
                ..OsdConfig::default()
            };
            if wide {
                cfg.width_px = 600;
                cfg.preview_font_pt = Some(17.0);
            }
            let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
            look.set_motion(&cfg.motion);
            for ups in [0, 1, 4] {
                let s = moments::edit_scenario(&look, ups);
                let popup = moments::popup_for(
                    moments::Moment::Edit,
                    Popup::scripted(look.clone(), &fonts),
                );
                let options = vo::Options {
                    fonts: fonts.clone(),
                    default_motion: default_motion(&look),
                    ..vo::Options::default()
                };
                let focus = s
                    .inputs
                    .iter()
                    .find(|(_, i)| matches!(i, vo::Input::KeyboardFocus { .. }))
                    .expect("the popup takes the keyboard")
                    .0;
                // The daemon releases the text 0.25 s after the last key.
                let last_key = s
                    .inputs
                    .iter()
                    .filter(|(_, i)| matches!(i, vo::Input::Key(_)))
                    .map(|(t, _)| *t)
                    .fold(focus, f64::max);
                let env = vo::envelope::Envelope {
                    transcript: 0..1 << 20,
                    from: focus,
                    until: last_key + 0.2,
                    ..vo::envelope::Envelope::default()
                };
                let report = vo::envelope::check_scenario(popup, &s, 30.0, options, &env);
                assert!(
                    report.is_ok() && report.glyphs_checked > 0,
                    "{name} {} with {ups} ups: {report}",
                    if wide { "wide" } else { "default" }
                );
            }
        }
    }

    #[test]
    fn swiss_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Swiss, "swiss");
    }

    #[test]
    fn swiss_edits_within_the_envelope() {
        envelope(OsdTheme::Swiss, "swiss");
    }

    #[test]
    fn dotmatrix_edits_within_the_envelope() {
        envelope(OsdTheme::Dotmatrix, "dotmatrix");
    }

    #[test]
    fn swissfi_edits_within_the_envelope() {
        envelope(OsdTheme::Swissfi, "swissfi");
    }

    #[test]
    fn dotmatrix_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Dotmatrix, "dotmatrix");
    }

    #[test]
    fn swissfi_edits_in_its_own_layout() {
        Rig::run(OsdTheme::Swissfi, "swissfi");
    }
}
