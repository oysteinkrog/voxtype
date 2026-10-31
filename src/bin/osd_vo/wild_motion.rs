//! Wildcard motion on the transcript (MO-83, MO-89, MO-81), from
//! `vo::scene::flow::letter_physics`, `vo::scene::flow::magnet` and
//! `vo::scene::font::var_axes`:
//!
//! - `letter-physics` (MO-83): new letters drop into the line as small rigid
//!   bodies, bounce twice, nudge their neighbours and lock into place, with
//!   faint motion vectors while they fly. On the reading line the drop is
//!   3 px at most, so the physics is subtle there.
//! - `magnetic-caret` (MO-89): the caret is a small magnet. New words snap
//!   to it along their line; its field lines brighten and tighten with the
//!   voice and relax into concentric rings in silence.
//! - `variable-breath` (MO-81): the transcript is set in Roboto Flex, a
//!   variable font, and each word gets its own instance: loudness sets the
//!   weight, tempo the width and pitch the slant, from what the voice did
//!   while the word was said (`vo::scene::word_voice`). The newest word also
//!   breathes with the live voice. On the reading line the axes stay inside
//!   the envelope (`Axes::reading`); the specimen tile shows the full ranges.
//!   Each word keeps the width the look's own font gives it
//!   (`Element::word_fonts_in_place`), so the breath changes how the letters
//!   are drawn and never moves a word or a line break.
//!
//! Each is a motion feature (`--motion`, `VOXTYPE_OSD_VO_MOTION`), off by
//! default. A look takes part by passing its transcript through
//! [`WildMotion::wild_motion`] last in its builder chain. Under
//! `reduced-motion` letters land in place, the field stays as still rings,
//! and each word keeps the axes it was said with, without breathing.

use super::*;

pub(crate) use vo::scene::flow::letter_physics::LetterPhysics;
use vo::scene::flow::magnet::Magnet;
use vo::scene::flow::Caret;
use vo::scene::font::var_axes::Axes;
use vo::scene::word_voice::WordVoices;
use vo_core::signals::Sig;
use vo_core::words::WordId;

use std::sync::Mutex;

use super::kit::at;

pub(crate) const LETTER_PHYSICS: &str = "letter-physics";
pub(crate) const MAGNETIC_CARET: &str = "magnetic-caret";
pub(crate) const VARIABLE_BREATH: &str = "variable-breath";

/// How much of the live voice the newest word takes over its own reading.
const BREATH: f32 = 0.6;

/// What the variable font keeps between views: each word's voice reading.
#[derive(Default)]
pub(crate) struct State(Mutex<WordVoices>);

/// The mock-ups' accent: motion vectors, field lines and the caret.
const ACCENT: u32 = 0x4DA3FF;

/// The wildcard motion features of a look on its transcript.
pub(crate) trait WildMotion {
    fn wild_motion(self, popup: &Popup) -> Self;
    /// [`WildMotion::wild_motion`] with the letters falling as `letters` says.
    fn wild_motion_with(self, popup: &Popup, letters: LetterPhysics) -> Self;
}

impl<M> WildMotion for Element<M> {
    fn wild_motion(self, popup: &Popup) -> Element<M> {
        self.wild_motion_with(popup, LetterPhysics::default())
    }

    fn wild_motion_with(self, popup: &Popup, letters: LetterPhysics) -> Element<M> {
        let look = &popup.look;
        let reduced = look.on("reduced-motion") || kit::reduced();
        let accent = Color::hex(ACCENT, 1.0);
        let mut words = self;
        let physics = look.on(LETTER_PHYSICS);
        // While a keyboard edit is pending or under way the edit layer draws
        // the one caret, so the end-of-text caret and its field are off.
        let magnet = look.on(MAGNETIC_CARET) && look.quiet.is_empty();
        if (physics || magnet) && !reduced {
            // The physics or the snap is the arrival: the word is readable at once.
            words = words.word_enter(Transition::fade().opacity(0.7));
        }
        if physics && !reduced {
            words = words.letter_physics(letters);
        }
        if magnet {
            words = words
                .caret(Caret { width: 2.0, gap: 3.0, color: Some(accent), gradient: None })
                .magnetic_caret(Magnet { still: reduced, reach: 1.4, ..Magnet::default().color(accent) });
        }
        if look.on(VARIABLE_BREATH) {
            let style = look.text_style();
            if let Some((_, fonts)) = popup.breath_fonts(style.size, false) {
                words = words.word_fonts_in_place(fonts);
            }
        }
        words
    }
}

impl Popup {
    /// Each shown word's axes now, from its voice reading, and the newest word's mixed with
    /// the live voice. Within the reading-line envelope unless `free`.
    fn breath_axes(&self, size: f32, free: bool) -> Vec<(WordId, Axes)> {
        let t = self.extra.now;
        let bus = self.signals.get();
        let mut voices = self.breath.0.lock().unwrap_or_else(|e| e.into_inner());
        voices.update(t, &self.model, &bus);
        let reduced = self.look.on("reduced-motion") || kit::reduced();
        // On room tone the newest word holds still: it mixes in silence at
        // its own pitch rather than the live level and pitch, which wander
        // on room tone (lq9).
        let room_tone = self.room_tone_pace(t).is_some();
        let level = if room_tone {
            0.0
        } else {
            bus.envelope(Sig::Level, 0.03, 0.3, t)
        };
        let live_pitch = voices.live_pitch(&bus, t);
        let n = self.shown.len();
        self.shown
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let v = voices.get(w.id);
                let letters = w.text.chars().count().max(1) as f32;
                let spl = v.map_or(0.0, |v| v.duration / letters);
                let said = v.map_or(Axes::REST, |v| Axes::from_voice(v.level.max(0.8 * v.peak), spl, v.pitch, size));
                let a = if i + 1 == n && !reduced {
                    let pitch = if room_tone {
                        v.map_or(live_pitch, |v| v.pitch)
                    } else {
                        live_pitch
                    };
                    said.mix(Axes::from_voice(level, spl, pitch, size), BREATH)
                } else {
                    said
                };
                (w.id, if free { a } else { a.reading() }.quantized())
            })
            .collect()
    }

    /// The flex font and each shown word's instance of it.
    pub(crate) fn breath_fonts(&self, size: f32, free: bool) -> Option<(u32, Vec<(WordId, u32)>)> {
        let fonts = &self.look.fonts;
        let base = fonts.asset("flex").ok()?;
        let words = self
            .breath_axes(size, free)
            .into_iter()
            .map(|(id, a)| (id, fonts.add_instance(base, &a.settings()).unwrap_or(base)))
            .collect();
        Some((base, words))
    }
}

/// The MO-81 station tile: a type specimen. The newest word large, in the
/// full axis ranges its voice gives it, with a readout of each axis and the
/// voice part it follows, over a matrix of the same word in fixed axis
/// states, the cell nearest the word's state lit.
pub(crate) fn breath_specimen(p: &Popup, _: &vo::Tokens) -> Element<Msg> {
    let size = p.look.surface();
    let accent = Color::hex(ACCENT, 1.0);
    let dim = Color::rgba(0.55, 0.6, 0.67, 1.0);
    let paper = Color::rgba(0.93, 0.94, 0.96, 1.0);
    let fonts = &p.look.fonts;
    let Ok(base) = fonts.asset("flex") else { return Element::column("breath") };
    let pad = 22.0;
    let inner = size.w - 2.0 * pad;
    let big = (0.17 * size.h).clamp(36.0, 72.0);
    let mono = |key: String, text: String, c: Color| Element::label(key, text).text_style(p.look.mono(9.0)).color(c);
    let word = p.shown.last().map_or("voice".to_string(), |w| w.text.clone());
    let now = p.breath_axes(big, true).last().map_or(Axes::REST, |a| a.1);
    let instance = |a: Axes| fonts.add_instance(base, &a.settings()).unwrap_or(base);
    let shown: Vec<Word> = p.shown.last().cloned().into_iter().collect();
    let hero = Element::live_text("hero", &shown)
        .text_style(TextStyle::new(base, big, 1.2 * big))
        .color(paper)
        .width(0.62 * inner)
        .word_fonts(shown.iter().map(|w| (w.id, instance(now))).collect());
    let readout = [
        ("WGHT", format!("{:.0}", now.wght), "loudness", (now.wght - 300.0) / 600.0),
        ("WDTH", format!("{:.0}", now.wdth), "tempo", (now.wdth - 75.0) / 50.0),
        ("OPSZ", format!("{:.0}", now.opsz), "size", (now.opsz - 8.0) / 64.0),
        ("SLNT", format!("{:.0}", now.slnt), "pitch", -now.slnt / 10.0),
    ];
    let mut panel = Element::panel("breath")
        .size(size.w, size.h)
        .radius(10.0)
        .fill(Color::rgba(0.04, 0.055, 0.075, 0.97))
        .child(at(mono("cap".into(), "VARIABLE FONT BREATHING  /  ROBOTO FLEX".into(), dim), pad, 12.0))
        .child(at(hero, pad, 30.0));
    let rx = pad + 0.66 * inner;
    for (i, (tag, value, from, k)) in readout.iter().enumerate() {
        let y = 34.0 + i as f32 * 17.0;
        panel = panel
            .child(at(mono(format!("tag{i}"), format!("{tag} {value:>4}"), paper), rx, y))
            .child(at(mono(format!("from{i}"), format!("<- {from}"), dim), rx + 0.2 * inner, y))
            .child(at(
                Element::panel(format!("bar{i}")).size(0.3 * inner * k.clamp(0.02, 1.0), 2.0).fill(accent),
                rx,
                y + 12.0,
            ));
    }
    // The matrix: columns from slow and soft to loud and fast, rows from low to high pitch.
    let cols = [(300.0, 75.0, "SLOW / SOFT"), (450.0, 100.0, "NEUTRAL"), (620.0, 115.0, "FAST / WIDE"), (850.0, 125.0, "LOUD / HIGH")];
    let rows = [(0.0, "UPRIGHT"), (-5.0, "SLNT -5"), (-10.0, "SLNT -10")];
    let top = 30.0 + 1.35 * big + 14.0;
    let label_w = 0.14 * inner;
    let cell_w = (inner - label_w) / cols.len() as f32;
    let cell_h = ((size.h - top - 10.0) / (rows.len() as f32 + 0.4)).min(48.0);
    let cell_px = (cell_h * 0.5).min(cell_w / (0.55 * word.chars().count().max(1) as f32)).clamp(9.0, 22.0);
    let dist = |a: &Axes| ((a.wght - now.wght) / 600.0).powi(2) + ((a.wdth - now.wdth) / 50.0).powi(2) + ((a.slnt - now.slnt) / 10.0).powi(2);
    let mut cells = Vec::new();
    for (c, &(w, d, name)) in cols.iter().enumerate() {
        panel = panel.child(at(mono(format!("col{c}"), name.into(), dim), pad + label_w + c as f32 * cell_w, top));
        for (r, &(s, _)) in rows.iter().enumerate() {
            cells.push((c, r, Axes { wght: w, wdth: d, opsz: vo::scene::font::var_axes::optical(cell_px), slnt: s }));
        }
    }
    let lit = cells.iter().min_by(|a, b| dist(&a.2).total_cmp(&dist(&b.2))).map(|c| (c.0, c.1));
    for (r, &(_, name)) in rows.iter().enumerate() {
        let y = top + 14.0 + r as f32 * cell_h;
        panel = panel.child(at(mono(format!("row{r}"), name.into(), dim), pad, y + 0.35 * cell_h));
    }
    for (c, r, a) in cells {
        let (x, y) = (pad + label_w + c as f32 * cell_w, top + 14.0 + r as f32 * cell_h);
        let on = lit == Some((c, r));
        let font = instance(a);
        panel = panel
            .child(at(
                Element::panel(format!("cell{c}{r}"))
                    .size(cell_w - 4.0, cell_h - 4.0)
                    .radius(3.0)
                    .fill(if on { accent.fade(0.16) } else { Color::rgba(1.0, 1.0, 1.0, 0.025) })
                    .motion(Prop::Fill, Spring::critical_to_98(0.25)),
                x,
                y,
            ))
            .child(at(
                Element::label(format!("w{c}{r}"), word.to_uppercase())
                    .text_style(TextStyle::new(font, cell_px, 1.2 * cell_px))
                    .color(if on { paper } else { paper.fade(0.55) }),
                x + 5.0,
                y + 0.5 * (cell_h - 4.0) - 0.62 * cell_px,
            ))
            .child(at(
                mono(format!("v{c}{r}"), format!("{:.0} {:.0} {:.0}", a.wght, a.wdth, a.slnt), dim.fade(0.8)),
                x + 5.0,
                y + cell_h - 15.0,
            ));
    }
    panel.signals(p.signals.clone())
}

/// The MO-83 station tile: the newest word as a large specimen that falls
/// freely from well above its baseline, with motion vectors, over the last
/// words of the transcript on a reading line, where the same physics stays
/// within the 3 px envelope.
pub(crate) fn letter_specimen(p: &Popup, _: &vo::Tokens) -> Element<Msg> {
    let size = p.look.surface();
    let reduced = p.look.on("reduced-motion");
    let accent = Color::hex(ACCENT, 1.0);
    let dim = Color::rgba(0.55, 0.6, 0.67, 1.0);
    let paper = Color::rgba(0.93, 0.94, 0.96, 1.0);
    let pad = 24.0;
    let inner = size.w - 2.0 * pad;
    let big = (0.2 * size.h).clamp(40.0, 84.0);
    let caption = |key: &'static str, text: &'static str| {
        Element::label(key, text).text_style(p.look.mono(10.0)).color(dim)
    };
    let newest: Vec<Word> = p.shown.last().cloned().into_iter().collect();
    let tail: Vec<Word> = p.shown.iter().rev().take(6).rev().cloned().collect();
    let mut specimen = Element::live_text("specimen", &newest)
        .text_style(TextStyle::new(SANS, big, 1.25 * big))
        .color(paper)
        .width(inner)
        .word_enter(Transition::fade().opacity(0.85));
    let mut reading = Element::live_text("reading", &tail)
        .text_style(p.look.text_style())
        .color(paper)
        .width(inner)
        .word_enter(Transition::fade().opacity(0.7));
    if !reduced {
        specimen = specimen.letter_physics(LetterPhysics::specimen(0.7 * big).vectors(accent.fade(0.8)));
        reading = reading.letter_physics(LetterPhysics::default());
    }
    let line = |key: &'static str, y: f32| {
        at(Element::panel(key).size(inner, 1.0).fill(dim.fade(0.5)), pad, y)
    };
    let spec_top = 0.12 * size.h + 0.7 * big;
    let read_top = spec_top + 1.45 * big + 34.0;
    Element::panel("letter-specimen")
        .size(size.w, size.h)
        .radius(10.0)
        .fill(Color::rgba(0.04, 0.055, 0.075, 0.97))
        .child(at(caption("cap-spec", "SPECIMEN  /  FREE FALL"), pad, 14.0))
        .child(at(specimen, pad, spec_top))
        .child(line("base-spec", spec_top + 0.93 * big))
        .child(at(caption("cap-read", "READING LINE  /  3 PX ENVELOPE"), pad, read_top - 22.0))
        .child(at(reading, pad, read_top))
        .signals(p.signals.clone())
}

#[cfg(test)]
mod tests {
    use super::super::edit_site::quiet_list;
    use super::super::wild_space::DEPTH_STACK;
    use super::*;
    use vo::scene::scene::Scene;
    use vo::scene::text::ShapedLayout;

    fn popup(theme: OsdTheme, features: &[&str]) -> (Popup, FontRegistry) {
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let cfg = OsdConfig {
            theme,
            ..OsdConfig::default()
        };
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        look.features = features.iter().map(|f| f.to_string()).collect();
        look.look_set = true;
        (Popup::new(look, &fonts), fonts)
    }

    /// The end-of-text caret of a live text set with `wild_motion`, or
    /// `None` when it draws none.
    fn end_caret(p: &Popup, fonts: &FontRegistry) -> Option<(vo::scene::geom::Rect, f32)> {
        let words: Vec<Word> = ["one", "two", "three"]
            .iter()
            .enumerate()
            .map(|(i, w)| Word {
                id: i as WordId,
                text: (*w).into(),
            })
            .collect();
        let el: Element<Msg> = Element::live_text("t", &words).width(400.0).wild_motion(p);
        let mut scene: Scene<Msg> =
            Scene::new().with_text(Box::new(ShapedLayout::new(fonts.clone())));
        scene.set_viewport(0.0, Size::new(400.0, 200.0), 1.0);
        scene.apply(el, 0.0);
        scene.caret(scene.find(&["t"]).unwrap(), 1.0)
    }

    /// R34 with M7.5: letter physics, variable breath and the depth stack
    /// are off while an edit is pending or under way, and the wild-motion
    /// end caret is hidden so only the edit caret shows.
    #[test]
    fn the_shared_effects_obey_edit_mode() {
        let wild = [LETTER_PHYSICS, VARIABLE_BREATH, MAGNETIC_CARET, DEPTH_STACK];
        for theme in [OsdTheme::Swissfi, OsdTheme::Showcase, OsdTheme::Jarvis] {
            let (mut p, fonts) = popup(theme, &wild);
            for f in wild {
                assert!(p.look.on(f), "{theme:?} {f}");
            }
            assert!(
                end_caret(&p, &fonts).is_some(),
                "{theme:?}: the end caret shows"
            );
            p.look.quiet = quiet_list(theme).into();
            for f in [LETTER_PHYSICS, VARIABLE_BREATH, DEPTH_STACK] {
                assert!(!p.look.on(f), "{theme:?} {f}");
            }
            assert!(
                end_caret(&p, &fonts).is_none(),
                "{theme:?}: no end caret while editing"
            );
            // Resume or a cancelled tap: everything comes back.
            p.look.quiet = Arc::from([]);
            assert!(wild.iter().all(|f| p.look.on(f)), "{theme:?}");
            assert!(end_caret(&p, &fonts).is_some(), "{theme:?}");
        }
    }
}
