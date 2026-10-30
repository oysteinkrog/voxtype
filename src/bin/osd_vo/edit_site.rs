//! How a look hosts the shared edit layer (plan section 4.4).
//!
//! The layer (`edit.rs`) owns what the text and the caret are; a look's
//! [`EditSite`] says where they go: the text box, how many lines can be
//! edited, the caret's style, its own effects to switch off, and how the
//! window's lines are placed. Every method has a default that fits a
//! left-aligned look with one live text, so most looks need nothing.
//!
//! From a pending tap until resume or cancel, the look's word motion is
//! the still profile and the [`QUIET`] features are off (R34): effects that
//! need live audio, move glyphs from their laid-out places, treat a change
//! as a model revision, or follow confidence.

use vo::scene::element::{Element, TextStyle};
use vo::scene::flow::{Caret, EditDecor};
use voxtype::osd::config::OsdTheme;

use super::edit::EditView;
use super::{Look, Msg};

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
        let style = self.caret(look);
        let decor = EditDecor {
            caret_style: style.caret,
            selection_color: Some(style.selection),
            mark_color: Some(style.mark),
            ..view.decor.clone()
        };
        el.map_live_texts(&mut |e| {
            e.live_source(view.text.clone())
                .edit(decor.clone())
                .still_motion(true)
        })
    }
}

/// The adapter of a look with nothing special.
pub(crate) struct DefaultSite;

impl EditSite for DefaultSite {}

/// The adapter of `theme`. Looks get their own as their edit beads land.
pub(crate) fn site(_theme: OsdTheme) -> &'static dyn EditSite {
    &DefaultSite
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
