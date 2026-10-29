//! Letter field effects (`vo_3d::glyph_fx`), each behind a feature:
//!
//! - `ink-diffusion` (MO-82): each new letter blooms like ink dropped on wet
//!   paper and settles into crisp type with a faint trace at its centre. The
//!   word arrives at 75 % ink and in place, so it reads from its first frame.
//! - `light-letters` (MO-84): the words as a stencil in front of a light
//!   that drifts across and follows the voice, with caustics and long soft
//!   shadows on the panel behind and the accent on the letters nearest it.
//! - `liquid-morph` (MO-87): a revised word flows into its correction;
//!   letters both versions share stay solid and slide, the changed ones
//!   blend as liquid. The live text skips the word while it flows.
//!
//! Under reduced motion each draws its settled state: the ink trace, still
//! light and shadows, the corrected word at once.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use vo::scene::flow::Revise;
use vo::scene::geom::Point;
use vo_3d::glyph_fx::{Effect, FxGlyph, GlyphFx, Ink, Light, Morph, Part, Side};
use vo_core::words::WordId;

pub(crate) const INK: &str = "ink-diffusion";
pub(crate) const LIGHT: &str = "light-letters";
pub(crate) const LIQUID: &str = "liquid-morph";

/// What the letter effects remember between views: when each word first
/// showed, its last text, and the words flowing into a new text now.
#[derive(Default)]
pub(crate) struct State(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    born: HashMap<WordId, f64>,
    texts: HashMap<WordId, String>,
    /// Old text and the time the change came.
    morphs: HashMap<WordId, (String, f64)>,
}

/// Pairs letters of `old` and `new` that a longest common subsequence
/// keeps: `(old index, new index)` in order.
fn kept_letters(old: &[char], new: &[char]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    let mut len = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i][j] = if old[i] == new[j] {
                len[i + 1][j + 1] + 1
            } else {
                len[i + 1][j].max(len[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if old[i] == new[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if len[i + 1][j] >= len[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// A word flowing into a new text: its id, old text and start time.
type FlowingWord = (WordId, String, f64);

impl Popup {
    fn letter_fx_on(&self) -> bool {
        let l = &self.look;
        l.on(INK) || l.on(LIGHT) || l.on(LIQUID)
    }

    fn letter_fx_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// Notes arrivals and revisions of the words on screen, and returns
    /// the words flowing into a new text now, with their old text and start.
    fn letter_fx_sync(&self) -> (HashMap<WordId, f64>, Vec<FlowingWord>) {
        let t = self.extra.now;
        let secs = Morph::default().secs as f64;
        let mut s = self.letter_fx.0.lock().unwrap_or_else(|e| e.into_inner());
        let s = &mut *s;
        for w in &self.shown {
            s.born.entry(w.id).or_insert(t);
            match s.texts.get(&w.id) {
                Some(old) if *old != w.text => {
                    if self.look.on(LIQUID) {
                        s.morphs.insert(w.id, (old.clone(), t));
                    }
                    s.texts.insert(w.id, w.text.clone());
                }
                Some(_) => {}
                None => {
                    s.texts.insert(w.id, w.text.clone());
                }
            }
        }
        // A finished morph stays drawn by the effect until the next view
        // hands the word back to the text.
        s.morphs.retain(|_, (_, at)| t - *at < secs + 0.05);
        let morphs = s
            .morphs
            .iter()
            .filter(|(id, _)| self.shown.iter().any(|w| w.id == **id))
            .map(|(id, (old, at))| (*id, old.clone(), *at))
            .collect();
        (s.born.clone(), morphs)
    }

    /// The live text with what the effects need: ink arrives in place at
    /// 75 % ink; the liquid morph swaps revised words at once and skips the
    /// ones it draws.
    pub(crate) fn letter_fx_words(&self, words: Element<Msg>) -> Element<Msg> {
        if !self.letter_fx_on() {
            return words;
        }
        let mut words = words;
        if self.look.on(INK) {
            words = words.word_enter(Transition::fade().opacity(0.75));
        }
        if self.look.on(LIQUID) {
            let (_, morphs) = self.letter_fx_sync();
            // Once the text is handed off it leaves as a whole: nothing is drawn elsewhere.
            let ids: Vec<u32> = if self.delivered().is_some() {
                Vec::new()
            } else {
                morphs.iter().map(|m| m.0).collect()
            };
            words = words.word_revise(Revise::Swap).words_drawn_elsewhere(&ids);
        }
        words
    }

    /// `text`, the column the live text is the first child of, with the
    /// effects under and over it. `width` is the live text's wrap width.
    pub(crate) fn letter_fx_layers(&self, text: Element<Msg>, width: f32) -> Element<Msg> {
        if !self.letter_fx_on() {
            return text;
        }
        let l = &self.look;
        let style = l.text_style();
        let height = self.lines as f32 * l.text_line;
        let reduced = self.letter_fx_reduced();
        let (born, morphs) = self.letter_fx_sync();
        let t = self.extra.now;
        let leaving = self.delivered().is_some();

        // Every glyph on screen with its word, from the layout the live
        // text uses.
        let joined = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(width));
        let mut starts = Vec::with_capacity(self.shown.len());
        let mut at = 0usize;
        for w in &self.shown {
            starts.push(at);
            at += w.text.len() + 1;
        }
        let word_of = |cluster: u32| starts.iter().rposition(|&s| s <= cluster as usize);
        let glyph = |g: &vo::scene::text::PlacedGlyph, start: f64| {
            FxGlyph::new(g.font, g.glyph, g.pos, style.size, l.foreground, start)
        };
        let fx = |effect, glyphs| {
            GlyphFx::new(effect, glyphs)
                .fonts(l.fonts.clone())
                .signals(self.signals.clone())
                .reduced(reduced)
                .item()
        };
        let place = |e: Element<Msg>| kit::at(e.size(width, height), 0.0, 0.0);

        let mut under = Vec::new();
        let mut over = Vec::new();
        if l.on(LIGHT) {
            // While the text is handed off it leaves the box, so the light keeps shining
            // but casts no letters, or it would show the old text as a ghost.
            let all: Vec<FxGlyph> = if leaving {
                Vec::new()
            } else {
                laid.glyphs.iter().map(|g| glyph(g, 0.0)).collect()
            };
            let light = Light {
                accent: l.accent,
                ..Light::default()
            };
            // The light dims away with the text it shone through.
            let dim = Spring::critical_to_98(0.45);
            under.push(place(
                Element::gpu(
                    "light-under",
                    fx(Effect::Light(light, Side::Under), all.clone()),
                )
                .opacity(if leaving { 0.0 } else { 1.0 })
                .motion(Prop::Opacity, dim),
            ));
            over.push(place(Element::gpu(
                "light-over",
                fx(Effect::Light(light, Side::Over), all),
            )));
        }
        if l.on(INK) && !leaving {
            let ink = Ink::default();
            let window = (ink.secs + ink.trace_secs) as f64;
            let fresh: Vec<FxGlyph> = laid
                .glyphs
                .iter()
                .filter_map(|g| {
                    let w = &self.shown[word_of(g.cluster)?];
                    let b = *born.get(&w.id)?;
                    (t - b < window).then(|| glyph(g, b))
                })
                .collect();
            if !fresh.is_empty() {
                over.push(place(Element::gpu("ink", fx(Effect::Ink(ink), fresh))));
            }
        }
        if l.on(LIQUID) && !leaving && !morphs.is_empty() {
            let mut glyphs = Vec::new();
            for (group, (id, old, start)) in morphs.iter().enumerate() {
                let Some(wi) = self.shown.iter().position(|w| w.id == *id) else {
                    continue;
                };
                let new: Vec<_> = laid
                    .glyphs
                    .iter()
                    .filter(|g| word_of(g.cluster) == Some(wi))
                    .collect();
                let Some(first) = new.first() else { continue };
                let alone = self.measure.layout(old, &style, None);
                let dx = first.pos.x - alone.glyphs.first().map_or(0.0, |g| g.pos.x);
                let olds: Vec<FxGlyph> = alone
                    .glyphs
                    .iter()
                    .map(|g| {
                        let p = Point::new(g.pos.x + dx, first.pos.y);
                        FxGlyph::new(g.font, g.glyph, p, style.size, l.foreground, *start)
                    })
                    .collect();
                let news: Vec<FxGlyph> = new.iter().map(|g| glyph(g, *start)).collect();
                let oc: Vec<char> = old.chars().collect();
                let nc: Vec<char> = self.shown[wi].text.chars().collect();
                // One glyph per letter in both, or the pairing would be wrong.
                let pairs = if oc.len() == olds.len() && nc.len() == news.len() {
                    kept_letters(&oc, &nc)
                } else {
                    Vec::new()
                };
                let g = group as u32;
                for (i, o) in olds.iter().enumerate() {
                    if !pairs.iter().any(|p| p.0 == i) {
                        glyphs.push(o.part(Part::Old, g));
                    }
                }
                for (j, n) in news.iter().enumerate() {
                    match pairs.iter().find(|p| p.1 == j) {
                        Some(&(i, _)) => {
                            glyphs.push(n.part(Part::Both, g).from_dx(olds[i].pos.x - n.pos.x))
                        }
                        None => glyphs.push(n.part(Part::New, g)),
                    }
                }
            }
            if !glyphs.is_empty() {
                over.push(place(Element::gpu(
                    "liquid",
                    fx(Effect::Morph(Morph::default()), glyphs),
                )));
            }
        }
        // The same tree with or without effects drawing, so the text node
        // stays when one starts or ends.
        Element::column("letter-fx")
            .children(under)
            .child(text)
            .children(over)
    }
}

#[cfg(test)]
mod tests {
    use super::kept_letters;

    #[test]
    fn kept_letters_follow_the_longest_common_run() {
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(kept_letters(&c("teh"), &c("tor")), vec![(0, 0)]);
        assert_eq!(
            kept_letters(&c("their"), &c("there")),
            vec![(0, 0), (1, 1), (2, 2), (4, 3)]
        );
        assert!(kept_letters(&c(""), &c("a")).is_empty());
    }
}
