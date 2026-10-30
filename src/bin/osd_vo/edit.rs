//! The shared edit layer (plan section 4.2): what a look draws while its
//! text is being edited.
//!
//! While the popup owns the text, the edit buffer replaces the preview as
//! the text source. The whole buffer goes through `WordModel::update`, so a
//! word keeps its id across edits and the looks' per-word state stays put.
//! The buffer is wrapped with the looks' own fitter (`ShapedLayout`) at the
//! look's text width and style, and a window of whole lines follows the
//! caret, scrolling as little as possible. Only the window's words go to the
//! look, with the window's text as the live source and an `EditDecor` for
//! the caret and selection, so the look's `Flow` draws them at its own
//! sprung glyph places. No cut and no ellipsis: what is shown is the buffer.

use std::ops::Range;

use vo::scene::element::TextStyle;
use vo::scene::flow::{Caret, EditDecor};
use vo::scene::text::{ShapedText, TextLayout};
use vo_core::edit::{EditBuffer, Selection};
use vo_core::words::Word;

/// What the look draws while editing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditView {
    /// The window's text, a slice of the buffer from the start of its first
    /// line to the end of its last; the look's live source.
    pub text: String,
    /// The window's first byte in the buffer.
    pub start: usize,
    /// Caret and selection in the window's bytes.
    pub decor: EditDecor,
    /// Lines in the window.
    pub lines: u32,
    /// The window line that holds the caret: the reading line (R35).
    pub caret_line: u32,
}

/// The layer's state for one edit: the window's place and the last layout.
#[derive(Debug, Default)]
pub(crate) struct EditLayer {
    /// First buffer line in the window; `None` until the first view, which
    /// puts the caret line in the newest-line slot.
    top: Option<usize>,
    /// The buffer text, style and width of the last layout, and the layout.
    shaped: Option<(String, TextStyle, f32, ShapedText)>,
    /// The buffer's revision and selection at the last view, and when they
    /// changed (the caret is solid for a moment after a change).
    seen: Option<(u64, Selection)>,
    since: f64,
}

/// The style the layer and the look's live text lay out with: the look's,
/// with caret stops and line bytes.
pub(crate) fn edit_style(style: TextStyle) -> TextStyle {
    TextStyle {
        stops: true,
        ..style
    }
}

impl EditLayer {
    /// Forget the window, for a new edit session.
    pub(crate) fn reset(&mut self) {
        *self = EditLayer::default();
    }

    /// The buffer laid out at `width`; laid out again only when the text,
    /// style or width changed.
    pub(crate) fn shaped(
        &mut self,
        text: &str,
        measure: &dyn TextLayout,
        style: TextStyle,
        width: f32,
    ) -> &ShapedText {
        let style = edit_style(style);
        let fresh = matches!(&self.shaped,
            Some((t, s, w, _)) if t == text && *s == style && *w == width);
        if !fresh {
            let shaped = measure.layout(text, &style, Some(width));
            self.shaped = Some((text.to_string(), style, width, shaped));
        }
        &self.shaped.as_ref().expect("laid out above").3
    }

    /// The view of `buffer` for a look with `max_lines` lines, at `t`.
    pub(crate) fn view(
        &mut self,
        buffer: &EditBuffer,
        measure: &dyn TextLayout,
        style: TextStyle,
        width: f32,
        max_lines: u32,
        t: f64,
    ) -> EditView {
        let sel = buffer.selection();
        if self.seen != Some((buffer.rev(), sel)) {
            self.seen = Some((buffer.rev(), sel));
            self.since = t;
        }
        let top = self.top;
        let shaped = self.shaped(buffer.text(), measure, style, width);
        let total = shaped.lines.len().max(1);
        let n = (max_lines.max(1) as usize).min(total);
        let caret_line = shaped.line_of(sel.focus, sel.affinity);
        let top = scroll(top, caret_line, n, total);
        let lines = &shaped.lines;
        let start = lines.get(top).map_or(0, |l| l.bytes.start);
        let end = lines
            .get(top + n - 1)
            .map_or(buffer.text().len(), |l| l.bytes.end);
        self.top = Some(top);
        let window = start..end;
        let local = |b: usize| b.clamp(start, end) - start;
        let selection = sel.range();
        let decor = EditDecor {
            caret: window.contains(&sel.focus).then(|| sel.focus - start).or(
                // The end of the last window line is a caret place too.
                (sel.focus == end).then_some(end - start),
            ),
            affinity: sel.affinity,
            selection: local(selection.start)..local(selection.end),
            since: self.since,
            caret_style: Caret {
                width: 2.0,
                ..Caret::default()
            },
            ..EditDecor::default()
        };
        EditView {
            text: buffer.text()[window].to_string(),
            start,
            decor,
            lines: n as u32,
            caret_line: (caret_line - top) as u32,
        }
    }
}

/// The first window line: the old one, moved as little as needed to hold
/// `caret_line` among `n` lines of `total`. With no old one, the caret line
/// is the window's last.
fn scroll(top: Option<usize>, caret_line: usize, n: usize, total: usize) -> usize {
    let top = top.unwrap_or_else(|| (caret_line + 1).saturating_sub(n));
    let top = if caret_line < top {
        caret_line
    } else if caret_line >= top + n {
        caret_line + 1 - n
    } else {
        top
    };
    top.min(total - n)
}

/// The byte range of each whitespace-separated word of `text`, in order:
/// the words `WordModel::update` makes of it.
pub(crate) fn word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push(s..i);
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

/// The model's words that lie in `window` of `text`, whose words they are.
pub(crate) fn window_words(words: &[Word], text: &str, window: Range<usize>) -> Vec<Word> {
    words
        .iter()
        .zip(word_ranges(text))
        .filter(|(_, r)| r.start >= window.start && r.end <= window.end)
        .map(|(w, _)| w.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vo::FontRegistry;
    use vo::ShapedLayout;
    use vo_core::edit::{Dir, EditAction, Unit};
    use vo_core::words::WordModel;

    fn layout() -> ShapedLayout {
        let theme = voxtype::osd::plasma::PlasmaTheme::default();
        let fonts: FontRegistry = crate::load_fonts(&theme).unwrap();
        ShapedLayout::new(fonts)
    }

    fn style() -> TextStyle {
        TextStyle::new(0, 16.0, 20.0)
    }

    fn lines_of(s: &ShapedText, text: &str) -> Vec<String> {
        s.lines
            .iter()
            .map(|l| text[l.bytes.clone()].to_string())
            .collect()
    }

    fn act(b: &mut EditBuffer, a: EditAction, measure: &dyn TextLayout, width: f32) {
        let s = measure.layout(b.text(), &edit_style(style()), Some(width));
        b.apply(a, &s, 0.0);
    }

    #[test]
    fn word_ids_survive_edits() {
        let mut model = WordModel::default();
        model.update("the quick brown fox");
        let before: Vec<_> = model.words().iter().map(|w| w.id).collect();
        let mut b = EditBuffer::new("the quick brown fox");
        b.set_selection(Selection::caret(9));
        b.apply_plain(EditAction::Insert("er".into()), 0.0);
        model.update(b.text());
        let after = model.words();
        assert_eq!(after[1].text, "quicker");
        assert_eq!(after[0].id, before[0]);
        assert_eq!(after[2].id, before[2]);
        assert_eq!(after[3].id, before[3]);
    }

    #[test]
    fn the_window_starts_with_the_caret_in_the_newest_slot_and_scrolls_minimally() {
        let measure = layout();
        let text = (0..60)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut b = EditBuffer::new(text.clone());
        let mut layer = EditLayer::default();
        let width = 200.0;
        let v = layer.view(&b, &measure, style(), width, 3, 0.0);
        let total = layer.shaped(&text, &measure, style(), width).lines.len();
        assert!(total > 6);
        assert_eq!((v.lines, v.caret_line), (3, 2));
        assert!(text.ends_with(&v.text));
        // Up within the window: no scroll.
        act(
            &mut b,
            EditAction::MoveLine {
                down: false,
                extend: false,
            },
            &measure,
            width,
        );
        let v2 = layer.view(&b, &measure, style(), width, 3, 0.0);
        assert_eq!((v2.start, v2.caret_line), (v.start, 1));
        // Up past the top: one line of scroll, caret on the top line.
        for _ in 0..2 {
            act(
                &mut b,
                EditAction::MoveLine {
                    down: false,
                    extend: false,
                },
                &measure,
                width,
            );
        }
        let v3 = layer.view(&b, &measure, style(), width, 3, 0.0);
        assert_eq!(v3.caret_line, 0);
        let shaped = layer.shaped(&text, &measure, style(), width);
        assert_eq!(v3.start, shaped.lines[total - 4].bytes.start);
        // Down again to the bottom line: no scroll until it passes it.
        act(
            &mut b,
            EditAction::MoveLine {
                down: true,
                extend: false,
            },
            &measure,
            width,
        );
        let v4 = layer.view(&b, &measure, style(), width, 3, 0.0);
        assert_eq!((v4.start, v4.caret_line), (v3.start, 1));
    }

    #[test]
    fn decor_bytes_are_window_bytes() {
        let measure = layout();
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let mut b = EditBuffer::new(text);
        b.set_selection(Selection {
            anchor: text.len() - 5,
            focus: text.len(),
            ..Selection::default()
        });
        let mut layer = EditLayer::default();
        let v = layer.view(&b, &measure, style(), 120.0, 2, 1.0);
        assert_eq!(&v.text[v.decor.selection.clone()], "kappa");
        assert_eq!(v.decor.caret, Some(v.text.len()));
        assert_eq!(v.decor.since, 1.0);
    }

    /// R54: the lines the look draws (its live text laid out from the
    /// window's text, at the same width and style) are the lines the layer
    /// uses for the window and for Up and Down. Random buffers, with runs
    /// of spaces and hard newlines, at several widths.
    #[test]
    fn window_lines_equal_the_full_layout() {
        let measure = layout();
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let vocab = [
            "a",
            "word",
            "longer",
            "ålesund",
            "x",
            "naïve",
            "supercalifragilistic",
            "I'm",
        ];
        for _ in 0..200 {
            let mut text = String::new();
            for _ in 0..rand(80) {
                text.push_str(vocab[rand(vocab.len() as u64) as usize]);
                text.push_str(match rand(10) {
                    0 => "\n",
                    1 => "  ",
                    _ => " ",
                });
            }
            let width = [90.0, 160.0, 320.0][rand(3) as usize];
            let mut b = EditBuffer::new(text.clone());
            b.set_selection(Selection::caret(rand(text.len() as u64 + 1) as usize));
            b.set_selection(b.selection());
            let mut layer = EditLayer::default();
            let max = 1 + rand(5) as u32;
            let v = layer.view(&b, &measure, style(), width, max, 0.0);
            let full = layer.shaped(&text, &measure, style(), width).clone();
            let full_lines = lines_of(&full, &text);
            let top = full
                .lines
                .iter()
                .position(|l| l.bytes.start == v.start)
                .unwrap();
            let shown = measure.layout(&v.text, &edit_style(style()), Some(width));
            let shown_lines = lines_of(&shown, &v.text);
            assert_eq!(
                shown_lines,
                full_lines[top..top + v.lines as usize],
                "text {text:?} width {width}"
            );
            // The caret line is in the window.
            assert!(v.caret_line < v.lines);
        }
    }

    #[test]
    fn window_words_are_the_models_words_in_the_window() {
        let mut model = WordModel::default();
        let text = "one  two\nthree four";
        model.update(text);
        let words = window_words(model.words(), text, 5..14);
        let names: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(names, ["two", "three"]);
        assert_eq!(word_ranges(text), [0..3, 5..8, 9..14, 15..19]);
    }

    #[test]
    fn nothing_moves_the_caret_off_a_short_text() {
        let measure = layout();
        let mut b = EditBuffer::new("");
        let mut layer = EditLayer::default();
        let v = layer.view(&b, &measure, style(), 200.0, 5, 0.0);
        assert_eq!(
            (v.text.as_str(), v.lines, v.caret_line, v.decor.caret),
            ("", 1, 0, Some(0))
        );
        b.apply_plain(EditAction::Insert("hi".into()), 0.0);
        b.apply_plain(
            EditAction::MoveBy {
                dir: Dir::Back,
                unit: Unit::Grapheme,
                extend: false,
            },
            0.0,
        );
        let v = layer.view(&b, &measure, style(), 200.0, 5, 0.0);
        assert_eq!(v.decor.caret, Some(1));
    }

    fn popup(dir: &tempfile::TempDir) -> crate::Popup {
        let cfg = voxtype::osd::config::OsdConfig::default();
        let plasma = voxtype::osd::plasma::PlasmaTheme::default();
        let fonts = crate::load_fonts(&plasma).unwrap();
        let look = crate::Look::new(&cfg, &plasma, String::new(), &fonts);
        let mut p = crate::Popup::new(look, &fonts);
        p.edit = Some(crate::edit_link::EditLink::new(crate::edit_link::Paths {
            buffer: dir.path().join("edit-buffer.json"),
            cmds: dir.path().join("edit-cmd"),
        }));
        p
    }

    fn block(phase: &str, frozen: &str) -> voxtype::osd::edit_ipc::EditBlock {
        voxtype::osd::edit_ipc::EditBlock {
            session: 1,
            phase: phase.into(),
            frozen: frozen.into(),
            tail: String::new(),
            pause: 1,
            raw_words: frozen.split_whitespace().count(),
        }
    }

    /// R53 and R55: a long dictation shown cut, with its "…", enters an
    /// edit: the buffer is the whole text and never the "…", the window's
    /// words are the shown words, and the text area keeps its lines at
    /// entry and at exit.
    #[test]
    fn entering_and_leaving_an_edit_keeps_the_size_and_the_text_whole() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut p = popup(&dir);
        let long = (0..200)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        p.files.preview = long.clone();
        p.text(&long);
        assert!(p.cut > 0, "the long text is shown cut");
        let lines = p.lines;
        p.edit
            .as_mut()
            .unwrap()
            .sync(Some(&block("editing", &long)));
        assert!(p.edit_text(0.0));
        let buffer = p
            .edit
            .as_ref()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .to_string();
        assert_eq!(buffer, long);
        assert!(!buffer.contains('\u{2026}'));
        let v = p.edit_view.clone().unwrap();
        assert!(!v.text.contains('\u{2026}'));
        let shown: Vec<_> = p.shown.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            shown.join(" "),
            v.text.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        assert_eq!(p.lines, lines, "same lines at entry");
        // The caret line is the newest line slot.
        assert_eq!(v.caret_line + 1, v.lines);
        // Resume: the edited text and the speech since, same lines.
        let mut listening = block("listening", &long);
        listening.raw_words = 200;
        p.files.live = Some(voxtype::osd::live::LiveDoc {
            edit: Some(listening.clone()),
            ..Default::default()
        });
        p.edit.as_mut().unwrap().sync(Some(&listening));
        assert!(!p.edit_text(0.0));
        assert!(p.edit_view.is_none());
        p.files.preview = format!("{long} and more");
        let source = p.source_text();
        assert!(source.ends_with("w199 and more"));
        p.text(&source);
        assert_eq!(p.lines, lines, "same lines at exit");
    }

    /// After a resume the look shows the edited text, not the preview's
    /// words before the boundary.
    #[test]
    fn after_a_resume_the_edited_text_is_shown() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut p = popup(&dir);
        let mut b = block("listening", "Hello, world.");
        b.raw_words = 2;
        p.files.live = Some(voxtype::osd::live::LiveDoc {
            edit: Some(b),
            ..Default::default()
        });
        p.files.preview = "helo world comma then more".into();
        assert_eq!(p.source_text(), "Hello, world. comma then more");
    }

    /// R42: one keystroke (retokenise, lay out, window and caret) in
    /// under 1 ms at 1,000 characters and 3 ms at 5,000. Run in release:
    /// `cargo test --release --features osd-vo --bin voxtype-osd-vo --
    /// --ignored keystroke_cost`.
    #[test]
    #[ignore]
    fn keystroke_cost() {
        let measure = layout();
        for (chars, limit_ms) in [(1_000usize, 1.0f64), (5_000, 3.0)] {
            let mut text = String::new();
            let mut i = 0;
            while text.len() < chars {
                text.push_str(&format!("word{i} "));
                i += 1;
            }
            let mut b = EditBuffer::new(text);
            let mut model = WordModel::default();
            let mut layer = EditLayer::default();
            model.update(b.text());
            layer.view(&b, &measure, style(), 400.0, 5, 0.0);
            let keys = 200;
            let start = std::time::Instant::now();
            for k in 0..keys {
                b.apply_plain(
                    EditAction::Insert(if k % 6 == 5 { " " } else { "x" }.into()),
                    k as f64,
                );
                model.update(b.text());
                let v = layer.view(&b, &measure, style(), 400.0, 5, k as f64);
                std::hint::black_box(window_words(
                    model.words(),
                    b.text(),
                    v.start..v.start + v.text.len(),
                ));
            }
            let per = start.elapsed().as_secs_f64() * 1e3 / keys as f64;
            println!("{chars} chars: {per:.3} ms per keystroke");
            assert!(
                per < limit_ms,
                "{chars} chars: {per:.3} ms per keystroke, limit {limit_ms} ms"
            );
        }
    }
}
