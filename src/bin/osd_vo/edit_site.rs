//! How a look hosts the shared edit layer (plan section 4.4).
//!
//! The layer (`edit.rs`) owns what the text and the caret are; a look's
//! [`EditSite`] says where they go: the text box, how many lines can be
//! edited, the caret's style, its own effects to switch off, and how the
//! window's lines are placed. Every method has a default that fits a
//! left-aligned look with one live text, so most looks need nothing.
//!
//! A look with its own adapter keeps it in its own file and gets an arm in
//! [`site`]. A look that draws one live text per line overrides `place`
//! with [`place_lines`] and builds its lines from
//! [`Popup::text_lines`](crate::Popup::text_lines).
//!
//! From a pending tap until resume or cancel, the look's word motion is
//! the still profile and the [`QUIET`] features are off (R34): effects that
//! need live audio, move glyphs from their laid-out places, treat a change
//! as a model revision, or follow confidence.

use std::collections::HashSet;
use std::ops::Range;

use vo::scene::element::{Element, TextStyle};
use vo::scene::flow::{Caret, EditDecor};
use vo_core::words::WordId;
use voxtype::osd::config::OsdTheme;

use super::edit::EditView;
use super::{Look, Msg, Popup};

/// Motion features off in every look while a tap is pending, the text is
/// settling or edited, or the edit is detached (plan section 4.3).
pub(crate) const QUIET: &[&str] = &[
    // Need live audio.
    super::model_signals::GHOST_TAIL,
    super::forming::FORMING,
    "pre-echo",
    "pause-marks",
    "exhale",
    "rewind",
    "audio-ruler",
    "sound-sculpture",
    "bowstring",
    "pitch-score",
    "spoken-tracking",
    // Move glyphs away from their laid-out places.
    "letter-physics",
    "variable-breath",
    "pitch-baseline",
    "margin-bell",
    "width-escrow",
    "fold-under",
    "depth-stack",
    "particle-source",
    "stroke-write",
    "beam-write",
    "spoken-stagger",
    "word-bulge",
    "lean-pace",
    "dot-sort",
    "unthread",
    // Treat a change as a model revision.
    "correction-hinge",
    "revision-address",
    // Follow confidence.
    "onset-ink",
    "confidence-ink",
    // Draw the glyphs as outlines of their own.
    super::glyph_paths::MORPH,
    super::glyph_paths::BREATH,
];

/// Where a look's editable text sits: the style and the wrap width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextBox {
    pub style: TextStyle,
    pub width: f32,
}

/// The caret, selection and edit-mark colours of a look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CaretStyle {
    pub caret: Caret,
    pub selection: vo::scene::geom::Color,
    pub mark: vo::scene::geom::Color,
}

pub(crate) trait EditSite {
    /// The text box. Default: the look's transcript style and width.
    fn text_box(&self, look: &Look) -> TextBox {
        TextBox {
            style: look.text_style(),
            width: look.text_width(),
        }
    }

    /// The text box in popup `p`, for a look whose box depends on the
    /// popup's state, such as Island's pill or card. Default:
    /// [`EditSite::text_box`] of the popup's look.
    fn text_box_in(&self, p: &Popup) -> TextBox {
        self.text_box(&p.look)
    }

    /// Editable lines in popup `p`. Default: [`EditSite::edit_lines`] of
    /// the popup's look.
    fn edit_lines_in(&self, p: &Popup) -> u32 {
        self.edit_lines(&p.look)
    }

    /// Editable lines in the window. Default: the look's `max_lines`.
    fn edit_lines(&self, look: &Look) -> u32 {
        look.max_lines()
    }

    /// Default: a 2 px bar in the accent colour, the selection in the
    /// accent at about 35 %, marks in the accent.
    fn caret(&self, look: &Look) -> CaretStyle {
        CaretStyle {
            caret: Caret {
                width: 2.0,
                color: Some(look.accent),
                ..Caret::default()
            },
            selection: look.accent.fade(0.35),
            mark: look.accent,
        }
    }

    /// The look's own features to switch off, beyond [`QUIET`].
    fn quiet(&self) -> &'static [&'static str] {
        &[]
    }

    /// Draw the window's lines. Default: every live text of the look shows
    /// the window, with its decor and the still profile.
    fn place(&self, look: &Look, el: Element<Msg>, view: &EditView) -> Element<Msg> {
        let decor = styled(view.decor.clone(), self.caret(look));
        el.map_live_texts(&mut |e| {
            e.live_source(view.text.clone())
                .edit(decor.clone())
                .still_motion(true)
        })
    }
}

/// `decor` in a look's caret, selection and mark colours.
fn styled(decor: EditDecor, style: CaretStyle) -> EditDecor {
    EditDecor {
        caret_style: style.caret,
        selection_color: Some(style.selection),
        mark_color: Some(style.mark),
        ..decor
    }
}

/// `place` for a look that draws one live text per line (Scope, Captions,
/// Karaoke, Jarvis, Synthwave). Each line node shows only its own line of
/// the window: the line's bytes as its source, the caret only on the caret
/// line, the selection and marks cut to the line, and the still profile. A
/// node is matched to the first unused window line with the same words;
/// live texts that hold no window line, such as a status row, are left as
/// they are. A line node with no words matches an empty line.
pub(crate) fn place_lines(
    site: &dyn EditSite,
    look: &Look,
    el: Element<Msg>,
    view: &EditView,
) -> Element<Msg> {
    let decor = styled(view.decor.clone(), site.caret(look));
    let lines: Vec<Vec<&str>> = view
        .line_bytes
        .iter()
        .map(|r| view.text[r.clone()].split_whitespace().collect())
        .collect();
    let mut used = vec![false; lines.len()];
    el.map_live_texts(&mut |e| {
        let words: Vec<String> = e
            .live_words()
            .map(|w| w.iter().map(|w| w.text.clone()).collect())
            .unwrap_or_default();
        let found = (0..lines.len()).find(|&j| !used[j] && lines[j] == words);
        let Some(j) = found else {
            return e;
        };
        used[j] = true;
        let bytes = view.line_bytes[j].clone();
        let has_caret = j == view.caret_line as usize;
        e.live_source(view.text[bytes.clone()].to_string())
            .edit(line_decor(&decor, bytes, has_caret))
            .still_motion(true)
    })
}

/// `decor` of the whole window for the line at `bytes`: in the line's own
/// bytes, with the caret only when the line holds it.
fn line_decor(decor: &EditDecor, bytes: Range<usize>, has_caret: bool) -> EditDecor {
    let local = |b: usize| b.clamp(bytes.start, bytes.end) - bytes.start;
    EditDecor {
        caret: decor.caret.filter(|_| has_caret).map(local),
        selection: local(decor.selection.start)..local(decor.selection.end),
        marks: decor
            .marks
            .iter()
            .map(|m| local(m.start)..local(m.end))
            .filter(|m| !m.is_empty())
            .collect(),
        ..decor.clone()
    }
}

/// After an edit until delivery (plan 4.1): every live text underlines its
/// `marked` words, in the look's mark colour, with no caret. A live text
/// with no marked word is left as it is.
pub(crate) fn place_marks(
    site: &dyn EditSite,
    look: &Look,
    el: Element<Msg>,
    marked: &HashSet<WordId>,
) -> Element<Msg> {
    let style = site.caret(look);
    el.map_live_texts(&mut |e| {
        // Without a live source a live text joins its words by one space.
        let mut at = 0;
        let mut marks = Vec::new();
        for w in e.live_words().unwrap_or_default() {
            if marked.contains(&w.id) {
                marks.push(at..at + w.text.len());
            }
            at += w.text.len() + 1;
        }
        if marks.is_empty() {
            return e;
        }
        e.edit(styled(
            EditDecor {
                marks,
                ..EditDecor::default()
            },
            style,
        ))
    })
}

/// The adapter of a look with nothing special.
pub(crate) struct DefaultSite;

impl EditSite for DefaultSite {}

/// The adapter of `theme`. A look with its own adapter gets an arm here;
/// the rest use [`DefaultSite`].
pub(crate) fn site(theme: OsdTheme) -> &'static dyn EditSite {
    match theme {
        OsdTheme::Captions => &super::captions::CaptionsSite,
        OsdTheme::Cyberpunk => &super::cyberpunk::CyberpunkSite,
        OsdTheme::Island => &super::island::IslandSite,
        OsdTheme::Palette => &super::palette::PaletteSite,
        OsdTheme::Particles => &super::particles::ParticlesSite,
        OsdTheme::Scope => &super::scope::ScopeSite,
        _ => &DefaultSite,
    }
}

/// Every feature `theme` switches off while editing.
pub(crate) fn quiet_list(theme: OsdTheme) -> Vec<&'static str> {
    QUIET.iter().chain(site(theme).quiet()).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::super::edit_link::{EditLink, Paths};
    use super::super::*;
    use super::*;
    use voxtype::osd::edit_ipc::EditBlock;

    fn popup(dir: &tempfile::TempDir, theme: OsdTheme) -> (Popup, FontRegistry) {
        let cfg = OsdConfig {
            theme,
            ..OsdConfig::default()
        };
        let plasma = PlasmaTheme::default();
        let fonts = load_fonts(&plasma).unwrap();
        let mut look = Look::new(&cfg, &plasma, String::new(), &fonts);
        look.set_motion(&cfg.motion);
        let mut p = Popup::new(look, &fonts);
        p.edit = Some(EditLink::new(Paths {
            buffer: dir.path().join("edit-buffer.json"),
            cmds: dir.path().join("edit-cmd"),
        }));
        (p, fonts)
    }

    fn sync(p: &mut Popup, phase: Option<&str>, frozen: &str) {
        let block = phase.map(|phase| EditBlock {
            session: 1,
            phase: phase.into(),
            frozen: frozen.into(),
            tail: String::new(),
            pause: 1,
            raw_words: frozen.split_whitespace().count(),
        });
        p.edit.as_mut().unwrap().sync(block.as_ref());
        p.sync_quiet();
        p.edit_text(1.0);
    }

    /// R34: the quiet list is off from a pending tap and back on after a
    /// cancelled tap or a resume.
    #[test]
    fn the_quiet_list_follows_the_edit() {
        let dir = tempfile::TempDir::new().unwrap();
        let (mut p, _) = popup(&dir, OsdTheme::Swiss);
        assert!(p.look.on("fold-under") && p.look.on("pitch-baseline"));
        sync(&mut p, Some("pending"), "");
        assert!(!p.look.on("fold-under") && !p.look.on("pitch-baseline"));
        // A cancelled tap: the daemon drops the block.
        sync(&mut p, None, "");
        assert!(p.look.on("fold-under"));
        for phase in ["pending", "settling", "editing", "detached", "editing"] {
            sync(&mut p, Some(phase), "some words");
            assert!(!p.look.on("fold-under"), "{phase}");
        }
        // Esc: the popup moves on to releasing at once, motion returns.
        p.edit.as_mut().unwrap().resume();
        p.sync_quiet();
        assert!(p.look.on("fold-under"));
        sync(&mut p, Some("listening"), "some words");
        assert!(p.look.on("fold-under"));
        // Every look's quiet list holds the shared one.
        for theme in OsdTheme::VO_LOOKS {
            let q = quiet_list(theme);
            assert!(QUIET.iter().all(|n| q.contains(n)), "{theme:?}");
        }
    }

    /// Outline glyphs asked for with `--motion` (glyph-morph, breath-line)
    /// are off while editing, so the edit draws plain glyphs.
    #[test]
    fn outline_glyphs_are_off_while_editing() {
        use super::super::glyph_paths::{BREATH, MORPH};
        let dir = tempfile::TempDir::new().unwrap();
        let (mut p, _) = popup(&dir, OsdTheme::Swiss);
        p.look.features = [MORPH, BREATH].iter().map(|f| f.to_string()).collect();
        assert!(p.look.on(MORPH) && p.look.on(BREATH));
        sync(&mut p, Some("pending"), "");
        assert!(!p.look.on(MORPH) && !p.look.on(BREATH));
        sync(&mut p, Some("editing"), "some words");
        assert!(!p.look.on(MORPH) && !p.look.on(BREATH));
        p.edit.as_mut().unwrap().resume();
        p.sync_quiet();
        assert!(p.look.on(MORPH) && p.look.on(BREATH));
    }

    #[test]
    fn a_lines_decor_is_cut_to_the_line() {
        let decor = EditDecor {
            caret: Some(12),
            selection: 3..14,
            marks: vec![0..2, 11..13, 20..22],
            ..EditDecor::default()
        };
        let line = line_decor(&decor, 10..16, true);
        assert_eq!(line.caret, Some(2));
        assert_eq!(line.selection, 0..4);
        assert_eq!(line.marks.len(), 1);
        assert_eq!(line.marks[0], 1..3);
        let other = line_decor(&decor, 0..10, false);
        assert_eq!(other.caret, None, "the caret on its own line only");
        assert_eq!(other.selection, 3..10);
        assert_eq!(other.marks.len(), 1);
        assert_eq!(other.marks[0], 0..2);
    }

    /// A look with one live text per line and a status row that is live
    /// text too: each line shows only its line, the caret stands on the
    /// caret line's glyphs, and the status row keeps its words.
    #[test]
    fn place_lines_gives_each_line_node_its_own_line() {
        use super::super::edit::{window_lines, window_words, EditLayer};
        use vo::scene::display::ItemKind;
        use vo::scene::scene::Scene;
        use vo::scene::text::ShapedLayout;
        use vo_core::words::WordModel;
        let dir = tempfile::TempDir::new().unwrap();
        let (p, fonts) = popup(&dir, OsdTheme::Scope);
        let measure = ShapedLayout::new(fonts.clone());
        let style = p.look.text_style();
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
        let mut model = WordModel::default();
        model.update(text);
        let mut b = vo_core::edit::EditBuffer::new(text);
        let at = text.find("theta").unwrap() + 2;
        b.set_selection(vo_core::edit::Selection::caret(at));
        let mut layer = EditLayer::default();
        let view = layer.view(&b, &measure, style, 260.0, 4, 1.0);
        assert!(view.lines >= 3, "{view:?}");
        let shown = window_words(
            model.words(),
            text,
            view.start..view.start + view.text.len(),
        );
        let lines = window_lines(&view, &shown);
        let mut status = WordModel::default();
        status.update("LISTENING now");
        let mut col = Element::column("lines");
        for (i, (words, _)) in lines.iter().enumerate() {
            col = col.child(Element::live_text(format!("line-{i}"), words).text_style(style));
        }
        col = col.child(Element::live_text("status", status.words()).text_style(style));
        let el = place_lines(&DefaultSite, &p.look, col, &view);

        let mut scene: Scene<Msg> =
            Scene::new().with_text(Box::new(ShapedLayout::new(fonts.clone())));
        scene.set_viewport(0.0, Size::new(600.0, 400.0), 1.0);
        scene.apply(el, 1.0);
        let dl = scene.sample(30.0);
        let mut glyphs = Vec::new();
        for (i, l) in dl.layers.iter().enumerate() {
            for it in dl
                .layer_items(i as _)
                .iter()
                .filter(|it| it.kind == ItemKind::Glyphs)
            {
                for g in &dl.glyphs[it.start as usize..(it.start + it.len) as usize] {
                    glyphs.push((g.word, l.transform.apply(g.pos)));
                }
            }
        }
        let id = |w: &str| model.words().iter().find(|x| x.text == w).unwrap().id;
        let theta: Vec<_> = glyphs.iter().filter(|g| g.0 == id("theta")).collect();
        let foot = dl.edit_caret.expect("the caret is drawn");
        let (min, max) = theta.iter().fold((f32::MAX, f32::MIN), |(a, b), g| {
            (a.min(g.1.x), b.max(g.1.x))
        });
        assert!(
            foot.x > min && foot.x < max,
            "{foot:?} inside theta {min}..{max}"
        );
        assert!(
            (foot.y - theta[0].1.y).abs() < 1.0,
            "{foot:?} on theta's baseline"
        );
        // Every window word and the status row's words are drawn once.
        let chars = |t: &str| t.chars().filter(|c| !c.is_whitespace()).count();
        assert_eq!(glyphs.len(), chars(&view.text) + chars("LISTENING now"));
    }

    /// The default adapter in Swiss: the look draws only the window's
    /// words, and the caret stands on the drawn glyphs: after the last glyph
    /// of the word before it, before the first of the word after, on their
    /// baseline.
    #[test]
    fn the_default_adapter_draws_swiss_with_the_caret_on_the_glyphs() {
        use vo::scene::display::ItemKind;
        use vo::scene::scene::Scene;
        use vo::scene::text::ShapedLayout;
        let dir = tempfile::TempDir::new().unwrap();
        let (mut p, fonts) = popup(&dir, OsdTheme::Swiss);
        let text = (0..80)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        sync(&mut p, Some("editing"), &text);
        // The caret after "w77", in the newest line.
        let at = text.find("w77").unwrap() + 3;
        let buffer = p.edit.as_ref().unwrap().buffer().unwrap().clone();
        let mut b = buffer.clone();
        b.set_selection(vo_core::edit::Selection::caret(at));
        let view = p.edit_layer.view(
            &b,
            &p.measure,
            p.look.text_style(),
            p.look.text_width(),
            5,
            1.0,
        );
        p.edit_view = Some(view.clone());
        let tokens = vo::Tokens::default();
        let mut scene: Scene<Msg> =
            Scene::new().with_text(Box::new(ShapedLayout::new(fonts.clone())));
        scene.set_viewport(0.0, p.look.live_surface(), 1.0);
        scene.apply(p.view(&tokens), 1.0);
        let dl = scene.sample(30.0);
        let id = |w: &str| p.model.words().iter().find(|x| x.text == w).unwrap().id;
        // Glyphs are in layer space and the caret foot in surface space.
        let mut glyphs = Vec::new();
        for (i, l) in dl.layers.iter().enumerate() {
            for it in dl
                .layer_items(i as _)
                .iter()
                .filter(|it| it.kind == ItemKind::Glyphs)
            {
                for g in &dl.glyphs[it.start as usize..(it.start + it.len) as usize] {
                    let q = l.transform.apply(g.pos);
                    glyphs.push((g.word, l.warp.as_ref().map_or(q, |w| w.apply(q))));
                }
            }
        }
        let drawn: std::collections::HashSet<u32> = glyphs.iter().map(|g| g.0).collect();
        assert!(drawn.contains(&id("w79")));
        assert!(!drawn.contains(&id("w0")), "only the window's words");
        let foot = dl.edit_caret.expect("the caret is drawn");
        let xs = |w: &str| {
            let g: Vec<_> = glyphs
                .iter()
                .filter(|g| g.0 == id(w))
                .map(|g| g.1)
                .collect();
            (
                g.iter().map(|g| g.x).fold(f32::MAX, f32::min),
                g.iter().map(|g| g.x).fold(f32::MIN, f32::max),
                g[0].y,
            )
        };
        let (_, before_max, base) = xs("w77");
        let (after_min, _, _) = xs("w78");
        assert!(
            foot.x > before_max && foot.x < after_min,
            "{foot:?} between {before_max} and {after_min}"
        );
        assert!(
            (foot.y - base).abs() < 1.0,
            "{foot:?} on the baseline {base}"
        );
    }
}
