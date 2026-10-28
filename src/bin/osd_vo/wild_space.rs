//! Three wildcard motion studies wired into looks, each behind a motion
//! feature (`--motion NAME` or a station exhibit):
//!
//! - `depth-stack` (MO-86, jarvis): older lines recede on glass sheets
//!   behind the flat reading line, with parallax as a new line arrives.
//! - `kinetic-lattice` (MO-85, scope): strands between the words whose bow
//!   is the pause before a word and whose weight is how loud it was.
//! - `sound-sculpture` (MO-88, captions): extruded letter bodies behind the
//!   flat text, as deep as the word was loud, laid flat when it is done.
//!
//! The framework pieces are `vo_scene::depth_stack`, `vo_scene::lattice`
//! and `vo_3d::Sculpture`. The reading line itself never moves: the stack
//! keeps its newest sheet flat, the lattice draws under the words, and the
//! sculpture's front faces sit exactly under the flat glyphs.

use super::*;

use std::sync::OnceLock;

use vo::scene::depth_stack::{DepthStack, Place, Sheet};
use vo::scene::geom::Point;
use vo::scene::lattice::{anchors, LatticeLook, LatticeWord};
use vo_3d::{Letter, SculptLook, Sculpture};
use vo_core::words::WordId;

pub(crate) const DEPTH_STACK: &str = "depth-stack";
pub(crate) const KINETIC_LATTICE: &str = "kinetic-lattice";
pub(crate) const SOUND_SCULPTURE: &str = "sound-sculpture";

/// The deepest a letter of the sculpture goes, in logical pixels, for the
/// loudest word.
const SCULPT_DEPTH: f32 = 34.0;

/// What the wildcard studies keep between views.
#[derive(Default)]
pub(crate) struct State {
    /// One sculpture per popup, so its glyph meshes upload once.
    sculpture: OnceLock<(Arc<Sculpture>, Arc<dyn vo::scene::gpu::GpuSource>)>,
}

impl Popup {
    pub(crate) fn wild_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// How loud a word was, 0 to 1: the loudest level over the sound it
    /// came from, or the level when it arrived.
    pub(crate) fn word_loudness(&self, id: WordId) -> f32 {
        let levels = self.kit_raw_levels();
        let Some(h) = self.heard.word(id) else {
            return 0.5;
        };
        let (a, b) = h.sound;
        let peak = (0..=8)
            .map(|i| levels.at(a + (b - a) * i as f64 / 8.0))
            .fold(0.0f32, f32::max);
        peak.clamp(0.0, 1.0)
    }

    /// The lattice timing of `words`: the pause before each, how loud it
    /// was and when it showed.
    pub(crate) fn lattice_words(&self, words: &[Word]) -> Vec<LatticeWord> {
        let mut prev_end: Option<f64> = None;
        words
            .iter()
            .map(|w| {
                let h = self.heard.word(w.id);
                let pause = match (h, prev_end) {
                    (Some(h), Some(end)) => (h.sound.0 - end).clamp(0.0, 3.0) as f32,
                    (Some(_), None) => 0.0,
                    _ => 0.25,
                };
                prev_end = h.map(|h| h.sound.1).or(prev_end);
                LatticeWord {
                    pause,
                    loud: self.word_loudness(w.id),
                    at: h.map_or(self.shown_at, |h| h.born),
                }
            })
            .collect()
    }

    /// The kinetic lattice under one line of `words` set in `style`, in a
    /// box `size` big laid out from the line's top left.
    pub(crate) fn lattice_line(
        &self,
        key: String,
        words: &[Word],
        style: &TextStyle,
        size: Size,
        look: &LatticeLook,
    ) -> Element<Msg> {
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        let nodes = anchors(&self.measure, &texts, style, None);
        let timed = self.lattice_words(words);
        look.view(key, size, timed.into(), nodes.into(), self.wild_reduced())
    }

    /// The depth stack of `lines` (oldest first, each with its width) in a
    /// box `w` wide and `rows` lines tall: the newest line flat at the
    /// bottom, the older ones receding on glass sheets above it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn depth_stack_lines(
        &self,
        lines: &[(Vec<Word>, f32)],
        w: f32,
        rows: u32,
        style: TextStyle,
        k: f32,
        text: Color,
        glow: Color,
    ) -> Element<Msg> {
        let reduced = self.wild_reduced();
        let line_h = style.line_height;
        let sheet = Size::new((w * 0.84).min(w - 24.0 * k), line_h + 8.0 * k);
        let stack = DepthStack {
            step_z: 90.0 * k,
            step_rise: line_h * 0.82,
            step_x: 12.0 * k,
            sheet: Some(Sheet {
                fill: Color::hex(0x07202a, 0.82),
                edge: glow.fade(0.45),
                edge_width: 1.0,
                radius: 4.0 * k,
                pad: (0.0, 4.0 * k),
            }),
            ..DepthStack::default()
        };
        let keyed: Vec<(vo::scene::Key, &(Vec<Word>, f32))> = lines
            .iter()
            .map(|l| {
                let key = l.0.first().map_or(0, |w| w.id);
                (vo::scene::Key::from(format!("sheet{key}")), l)
            })
            .collect();
        let content = |line: &&(Vec<Word>, f32), d: usize, p: &Place| {
            let (words, width) = *line;
            let lw = width.ceil() + 2.0;
            kit::at(
                Element::live_text("words", words)
                    .text_style(style)
                    .color(text)
                    .text_halo(glow.fade(0.45), 7.0 * k)
                    .text_blur(p.blur)
                    .motion(Prop::TextFx, stack.motion(d, reduced))
                    .size(lw, line_h)
                    .word_enter(Transition::fade().opacity(0.7).offset(0.0, 3.0))
                    .word_change(Transition::fade().opacity(0.7)),
                ((sheet.w - lw) / 2.0).max(0.0),
                4.0 * k,
            )
        };
        let h = rows.max(1) as f32 * line_h;
        let x = (w - sheet.w) / 2.0;
        kit::at(
            stack.view(
                "depth-stack",
                Size::new(sheet.w, h),
                sheet,
                &keyed,
                content,
                reduced,
            ),
            x,
            0.0,
        )
    }

    fn sculpture(&self, font: u32) -> Option<&(Arc<Sculpture>, Arc<dyn vo::scene::gpu::GpuSource>)> {
        if let Some(s) = self.wild.sculpture.get() {
            return Some(s);
        }
        let face = self.look.fonts.get(font)?;
        let mut look = SculptLook::default();
        if self.wild_reduced() {
            // Still: each letter stands at its depth at once.
            look.grow_secs = 0.001;
            look.flatten_spread = 0.0;
        }
        let s = Arc::new(Sculpture::new(
            face,
            font,
            vo_3d::sculpture::DEFAULT_CHARSET,
            look,
        ));
        let source = s.source();
        Some(self.wild.sculpture.get_or_init(|| (s, source)))
    }

    /// The sound sculpture under `rows` of text in `style`: each row's
    /// words and the top left of its text box, the last row the reading
    /// line. Letters of older rows lie flat; on the reading line each
    /// letter is as deep as its word was loud. `moves` is the spring the
    /// rows move on, so the bodies follow them. `done` lays all flat.
    pub(crate) fn sculpture_element(
        &self,
        rows: &[(Vec<Word>, Point)],
        style: &TextStyle,
        size: Size,
        moves: Spring,
        done: bool,
    ) -> Option<Element<Msg>> {
        let (sculpture, source) = self.sculpture(style.font)?;
        let t = self.extra.now;
        if done {
            sculpture.flatten(t);
        } else {
            let mut letters = Vec::new();
            for (r, (words, origin)) in rows.iter().enumerate() {
                let newest = r + 1 == rows.len();
                let text = words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let shaped = self.measure.layout(&text, style, None);
                let mut starts = Vec::with_capacity(words.len());
                let mut at = 0usize;
                for w in words {
                    starts.push(at);
                    at += w.text.len() + 1;
                }
                for g in &shaped.glyphs {
                    let c = g.cluster as usize;
                    let wi = starts.iter().rposition(|&s| s <= c).unwrap_or(0);
                    let word = &words[wi];
                    let depth = if newest {
                        SCULPT_DEPTH * self.word_loudness(word.id).max(0.15)
                    } else {
                        0.0
                    };
                    letters.push(Letter {
                        glyph: g.glyph,
                        font: g.font,
                        pos: Point::new(origin.x + g.pos.x, origin.y + g.pos.y),
                        size: style.size,
                        key: (word.id as u64) << 24 | ((c - starts[wi]) as u64) << 8 | (g.glyph as u64 & 0xff),
                        depth,
                        run: words.first().map_or(0, |w| w.id as u64),
                    });
                }
            }
            sculpture.set(t, &letters, Some(moves));
        }
        Some(kit::at(Element::gpu("sculpture", source.clone()).size(size.w, size.h), 0.0, 0.0))
    }
}
