//! Three wildcard motion studies wired into looks, each behind a motion
//! feature (`--motion NAME` or a station exhibit):
//!
//! - `depth-stack` (MO-86, jarvis): older lines recede on glass sheets
//!   behind the flat reading line, with parallax as a new line arrives.
//! - `kinetic-lattice` (MO-85, scope): strands between the words whose bow
//!   is the pause before a word and whose weight is how loud it was. LT-04
//!   feeds it the voice: pitch colours a bundle, punctuation gives it
//!   sentences and commas, onsets, arrivals and revisions pluck it, the level
//!   over room tone pulls the newest line taut, and while the popup listens a
//!   live strand runs from the newest word to the caret.
//! - `sound-sculpture` (MO-88, captions): extruded letter bodies behind the
//!   flat text, as deep as the word was loud, laid flat when it is done.
//!
//! The framework pieces are `vo_scene::depth_stack`, `vo_scene::lattice`
//! and `vo_3d::Sculpture`. The reading line itself never moves: the stack
//! keeps its newest sheet flat, the lattice draws under the words, and the
//! sculpture's front faces sit exactly under the flat glyphs.

use super::*;

use std::cell::RefCell;
use std::sync::OnceLock;

use vo::scene::depth_stack::{DepthStack, Place, Sheet};
use vo::scene::geom::Point;
use vo::scene::emitter::LevelTrack;
use vo::scene::lattice::{anchors, Boundary, LatticeLook, LatticeWord, LiveStrand, Pluck};
use vo_3d::{Letter, SculptLook, Sculpture};
use vo_core::words::WordId;

pub(crate) const DEPTH_STACK: &str = "depth-stack";
pub(crate) const KINETIC_LATTICE: &str = "kinetic-lattice";
pub(crate) const SOUND_SCULPTURE: &str = "sound-sculpture";

/// The deepest a letter of the sculpture goes, in logical pixels, for the
/// loudest word.
const SCULPT_DEPTH: f32 = 34.0;

/// Peak swing, in logical pixels, of the pluck a word's arrival gives its
/// bundle: this times 0.5 for a quiet word up to 1.5 for the loudest.
const ARRIVAL_PLUCK: f32 = 2.0;
/// Peak swing of the pluck an onset gives the bundle the voice is on.
const ONSET_PLUCK: f32 = 1.2;
/// The lattice's level moves by at least this much before it takes a new
/// entry, so the track stays short and room tone adds none.
const LEVEL_STEP: f32 = 0.04;
/// How fast the lattice's level follows the voice (about 4 / rate seconds).
const LEVEL_RATE: f32 = 14.0;
/// Seconds of level history the lattice keeps.
const LEVEL_KEEP: f64 = 2.0;

/// What the wildcard studies keep between views.
#[derive(Default)]
pub(crate) struct State {
    /// One sculpture per popup, so its glyph meshes upload once.
    sculpture: OnceLock<(Arc<Sculpture>, Arc<dyn vo::scene::gpu::GpuSource>)>,
    /// The voice as the lattice hears it, fed from the raw levels at each view.
    voice: RefCell<LatticeVoice>,
}

/// The level over room tone as a [`LevelTrack`], and when the voice was last
/// heard over room tone.
struct LatticeVoice {
    level: LevelTrack,
    /// Scene time of the newest level frame fed.
    fed: f64,
    /// The level last pushed: 0 on room tone.
    sent: f32,
    /// Scene time of the newest frame over room tone.
    voiced: f64,
}

impl Default for LatticeVoice {
    fn default() -> LatticeVoice {
        let mut level = LevelTrack::new(LEVEL_RATE);
        level.keep = LEVEL_KEEP;
        LatticeVoice { level, fed: f64::NEG_INFINITY, sent: 0.0, voiced: f64::NEG_INFINITY }
    }
}

impl LatticeVoice {
    /// Feeds the frames of `levels` newer than the last fed. A level under
    /// `floor` (room tone) counts as 0, and a new entry is pushed only when
    /// the level moves by `LEVEL_STEP` or crosses the floor, so on room tone
    /// the track stops growing and the lattice holds still.
    fn feed(&mut self, levels: &kit::Levels, floor: f32) {
        let n = levels.levels.len();
        if n == 0 {
            return;
        }
        if levels.newest_at < self.fed {
            // A new recording: start again.
            *self = LatticeVoice::default();
        }
        let step = 1.0 / FRAME_HZ as f64;
        for (i, &l) in levels.levels.iter().enumerate() {
            let at = levels.newest_at - (n - 1 - i) as f64 * step;
            if at <= self.fed + step * 0.5 {
                continue;
            }
            let l = if l > floor { l } else { 0.0 };
            if l > 0.0 {
                self.voiced = at;
            }
            if (l - self.sent).abs() >= LEVEL_STEP || ((l > 0.0) != (self.sent > 0.0)) {
                self.level.push(at, l);
                self.sent = l;
            }
            self.fed = at;
        }
    }
}

/// What ends `text`: a sentence at `.`, `?`, `!` or `…`, a comma at `,`,
/// `;` or `:`, ignoring closing quotes and brackets.
pub(crate) fn boundary(text: &str) -> Boundary {
    match text
        .trim_end_matches(['"', '\'', ')', ']', '»', '”', '’'])
        .chars()
        .next_back()
    {
        Some('.' | '?' | '!' | '…') => Boundary::Sentence,
        Some(',' | ';' | ':') => Boundary::Comma,
        _ => Boundary::None,
    }
}

/// The plucks of a line of `words`: each arrival plucks its own bundle, as
/// hard as the word was loud, and each onset in `onsets` plucks the bundle
/// the voice is on (the newest word shown by then). Revisions are plucked by
/// the lattice itself from `LatticeWord::changed`. In time order.
pub(crate) fn lattice_plucks(words: &[LatticeWord], onsets: &[f64]) -> Vec<Pluck> {
    let mut out: Vec<Pluck> = words
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, w)| Pluck { at: w.at, bundle: i, amp: ARRIVAL_PLUCK * (0.5 + w.loud.clamp(0.0, 1.0)) })
        .collect();
    let first = words.first().map_or(f64::INFINITY, |w| w.at);
    for &o in onsets.iter().filter(|&&o| o > first) {
        // The newest word shown by the onset, whose bundle the voice is on.
        let on = words.iter().rposition(|w| w.at <= o).unwrap_or(0);
        if on > 0 {
            out.push(Pluck { at: o, bundle: on, amp: ONSET_PLUCK });
        }
    }
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
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

    /// The lattice timing of `words`: the pause before each (from the
    /// engine's word times when they fit, else from arrival order), how loud
    /// it was, when it showed and last changed, its pitch and what ends it.
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
                    pitch: h.and_then(|h| h.pitch),
                    changed: h.and_then(|h| h.changed),
                    boundary: boundary(&w.text),
                }
            })
            .collect()
    }

    /// The level under which the lattice hears room tone: the popup's
    /// room-tone floor (RD-11) on the raw level scale.
    fn lattice_floor(&self) -> f32 {
        tick_level(10f64.powf(ROOM_TONE_DBFS / 20.0), self.look.gain) as f32
    }

    /// The kinetic lattice under one line of `words` set in `style`, in a
    /// box `size` big laid out from the line's top left. `look` brings the
    /// style; the voice is filled in here. `caret`, for the newest line
    /// while the popup listens, is the caret's x in the box: the level then
    /// pulls this line taut and a live strand runs from its newest word to
    /// the caret, slackening over 3 s of silence.
    pub(crate) fn lattice_line(
        &self,
        key: String,
        words: &[Word],
        style: &TextStyle,
        size: Size,
        look: &LatticeLook,
        caret: Option<f32>,
    ) -> Element<Msg> {
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        let nodes = anchors(&self.measure, &texts, style, None);
        let timed = self.lattice_words(words);
        let mut look = look.clone();
        look.plucks = lattice_plucks(&timed, self.heard.onsets()).into();
        if let (Some(caret_x), Some(newest)) = (caret, timed.len().checked_sub(1)) {
            let floor = self.lattice_floor();
            let mut voice = self.wild.voice.borrow_mut();
            voice.feed(&self.kit_raw_levels(), floor);
            look.level = Some(voice.level.clone());
            look.level_floor = floor;
            look.live = Some(LiveStrand {
                from_node: newest,
                caret_x,
                since: voice.voiced.max(timed[newest].at),
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_ends_sentences_and_clauses() {
        assert_eq!(boundary("done."), Boundary::Sentence);
        assert_eq!(boundary("really?\""), Boundary::Sentence);
        assert_eq!(boundary("first,"), Boundary::Comma);
        assert_eq!(boundary("note:"), Boundary::Comma);
        assert_eq!(boundary("popup"), Boundary::None);
        assert_eq!(boundary("v1.2"), Boundary::None);
    }

    #[test]
    fn arrivals_and_onsets_pluck_the_right_bundles_in_order() {
        let words = [
            LatticeWord::new(0.0, 0.2, 1.0),
            LatticeWord::new(0.1, 1.0, 1.5),
            LatticeWord::new(0.1, 0.0, 2.0),
        ];
        // One onset before the line, one on word 1, one on word 2.
        let p = lattice_plucks(&words, &[0.5, 1.7, 2.4]);
        let got: Vec<(f64, usize)> = p.iter().map(|p| (p.at, p.bundle)).collect();
        assert_eq!(got, [(1.5, 1), (1.7, 1), (2.0, 2), (2.4, 2)]);
        // The louder word plucks harder.
        assert!(p[0].amp > p[2].amp);
        assert_eq!(p[1].amp, ONSET_PLUCK);
    }

    fn levels(values: &[f32], newest_at: f64) -> kit::Levels {
        kit::Levels { levels: values.into(), newest_at }
    }

    #[test]
    fn room_tone_adds_nothing_to_the_level_track() {
        let floor = 0.1;
        let mut v = LatticeVoice::default();
        let step = 1.0 / FRAME_HZ as f64;
        // Speech then room tone.
        let mut all: Vec<f32> = vec![0.6; 20];
        all.extend([0.03; 30]);
        v.feed(&levels(&all, 1.0), floor);
        let last = v.level.last();
        let voiced = v.voiced;
        assert!((voiced - (1.0 - 30.0 * step)).abs() < 1e-9);
        assert!(v.level.at(5.0).abs() < 1e-3, "room tone counts as no voice");
        // More room tone, fed in pieces as views come: nothing new.
        for k in 1..=50 {
            all.push(0.02 + 0.01 * (k % 3) as f32);
            all.remove(0);
            v.feed(&levels(&all, 1.0 + k as f64 * step), floor);
        }
        assert_eq!(v.level.last(), last);
        assert_eq!(v.voiced, voiced);
        // The voice again: it is heard at once.
        all.push(0.7);
        all.remove(0);
        v.feed(&levels(&all, 1.0 + 51.0 * step), floor);
        assert!(v.level.last() > last);
        assert!((v.voiced - (1.0 + 51.0 * step)).abs() < 1e-9);
    }

    #[test]
    fn a_new_recording_starts_the_voice_again() {
        let mut v = LatticeVoice::default();
        v.feed(&levels(&[0.6; 10], 9.0), 0.1);
        v.feed(&levels(&[0.03; 10], 1.0), 0.1);
        assert_eq!(v.voiced, f64::NEG_INFINITY);
    }
}
