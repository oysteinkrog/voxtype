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
//!   live strand runs from the newest word to the caret. LT-09: after 3 s
//!   of silence a lattice with gutters exhales (every bow eases out to 1.3x
//!   over 1.2 s and snaps back at the next onset), and `lattice-drift` lets the older
//!   lines' lattices wander a pixel or two at 15 steps a second, clipped to
//!   their own line so the newest line's box never changes.
//! - `sound-sculpture` (MO-88, captions): extruded letter bodies behind the
//!   flat text, as deep as the word was loud, laid flat when it is done.
//!
//! The framework pieces are `vo_scene::depth_stack`, `vo_scene::lattice`
//! and `vo_3d::Sculpture`. The reading line itself never moves: the stack
//! keeps its newest sheet flat, the lattice draws under the words, and the
//! sculpture's front faces sit exactly under the flat glyphs.

use super::*;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use vo::scene::depth_stack::{DepthStack, Hold, Place, Sheet};
use vo::scene::geom::Point;
use vo::scene::emitter::LevelTrack;
use vo::scene::lattice::{
    anchors, Boundary, LatticeDrift, LatticeLook, LatticeStyle, LatticeWord, LineGutter, LiveStrand,
    Pluck, Quiet,
};
use vo_3d::{Letter, SculptLook, Sculpture};
use vo_core::words::WordId;

pub(crate) const DEPTH_STACK: &str = "depth-stack";
pub(crate) const KINETIC_LATTICE: &str = "kinetic-lattice";
pub(crate) const SOUND_SCULPTURE: &str = "sound-sculpture";
/// LT-09: the older lines' lattices drift (opt-in, with `kinetic-lattice`).
pub(crate) const LATTICE_DRIFT: &str = "lattice-drift";

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
/// How far a line's glyphs reach under the baseline and over it (ascenders),
/// as a share of the font size, for the gutters between lines (LT-07).
const DESCENT: f32 = 0.25;
const ASCENT: f32 = 0.74;
/// Words whose loudness is kept before the ones no longer heard are dropped.
const LOUDNESS_KEEP: usize = 256;

/// How much of its colour an older line's lattice keeps.
const OLDER_FADE: f32 = 0.5;

/// How a look runs the kinetic lattice under its wrapped transcript (LT-07):
/// a preset in the look's two colours, at a scale, with the room under the
/// newest line its strands may use.
#[derive(Clone, Copy)]
pub(crate) struct LatticeSet {
    pub(crate) preset: LatticeStyle,
    /// Strands, and the newest node and strands.
    pub(crate) color: Color,
    pub(crate) accent: Color,
    /// Lengths and widths times this.
    pub(crate) scale: f32,
    /// The pause labels on the newest line, for presets that show them.
    pub(crate) label: Option<TextStyle>,
    /// Pixels under the newest line's box that its strands may use: the
    /// gutter LT-09's exhale needs.
    pub(crate) gutter: f32,
    /// The lattice's opacity while it stays (a look dims its words on a
    /// cancel); it fades to 0 as it leaves.
    pub(crate) opacity: f32,
}

/// The gutters of one line of the lattice (LT-07), in its own box with the
/// line's `baseline`: the newest line's strip runs from its descenders to
/// `gutter` under its box and bows down only; an older line's is the gap
/// between its descenders and the next line's ascenders.
pub(crate) fn line_gutter(style: &TextStyle, baseline: f32, newest: bool, gutter: f32) -> LineGutter {
    let descent = style.size * DESCENT;
    let top = baseline + descent;
    let height = if newest {
        (style.line_height - top).max(0.0) + gutter
    } else {
        (style.line_height - style.size * (ASCENT + DESCENT)).max(0.0)
    };
    LineGutter { top, height, newest, descent }
}

/// `look` fitted to `g`: on an older line the drop lines and ticks stay in
/// half the gutter; on the newest a label shows only when it fits under its
/// drop line, which shortens to make room.
pub(crate) fn fit_look(mut look: LatticeLook, g: &LineGutter) -> LatticeLook {
    let half = (g.height * 0.5 - 1.0).max(0.0);
    if !g.newest {
        // An older line stays quiet: fainter, finer, no accent, so only the
        // newest line's lattice reads as live.
        look.color = look.color.fade(OLDER_FADE);
        look.accent = look.color;
        look.width = (look.width.0 * 0.7, look.width.1 * 0.7);
        look.node_radius *= 0.7;
        look.stem = look.stem.min(half);
        look.tick = look.tick.min(half);
        look.label = None;
        return look;
    }
    let node = g.top + look.drop;
    let room = g.top + g.height - node;
    look.tick = look.tick.min((node - g.top).max(0.0));
    if let Some(label) = look.label {
        let stem = look.stem.min(room - label.line_height - 1.0);
        if stem >= 2.0 {
            look.stem = stem;
        } else {
            look.label = None;
        }
    }
    if look.label.is_none() {
        look.stem = look.stem.min(room.max(0.0));
    }
    look
}

/// What the wildcard studies keep between views.
#[derive(Default)]
pub(crate) struct State {
    /// One sculpture per popup, so its glyph meshes upload once.
    sculpture: OnceLock<(Arc<Sculpture>, Arc<dyn vo::scene::gpu::GpuSource>)>,
    /// The voice as the lattice hears it, fed from the raw levels at each view.
    voice: RefCell<LatticeVoice>,
    /// Each word's loudness and the sound it was measured over, kept once
    /// the whole sound was in the level history, so a word does not go
    /// quiet when its sound scrolls out of that history (LT-12).
    loudness: RefCell<HashMap<WordId, KeptLoudness>>,
}

/// A word's loudness and the sound (start, end) it was measured over.
type KeptLoudness = ((f64, f64), f32);

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

/// The silence a line of `words` exhales in: since the voice was last
/// heard over room tone (`voiced`) or the line's newest word, whichever is
/// later. Only for a look with gutters: with none a 1.3x bow reaches up
/// into the newest line's letters and down into the next line's ascenders.
fn lattice_quiet(look: &LatticeLook, words: &[LatticeWord], voiced: f64) -> Option<Quiet> {
    let line_at = words.iter().map(|w| w.at).fold(f64::NEG_INFINITY, f64::max);
    (line_at.is_finite() && !look.gutters.is_empty()).then(|| Quiet::since(voiced.max(line_at)))
}

/// A drift seed from a lattice's key, so lines side by side wander apart.
fn drift_seed(key: &str) -> u32 {
    key.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

impl Popup {
    pub(crate) fn wild_reduced(&self) -> bool {
        self.look.on("reduced-motion") || kit::reduced()
    }

    /// How loud a word was, 0 to 1: the loudest level over the sound it
    /// came from. Measured once the whole sound is in the level history and
    /// kept from then on: the history is a few seconds long, and a word
    /// measured after its sound scrolled out would read as silent, so its
    /// strand would thin seconds after it landed.
    pub(crate) fn word_loudness(&self, id: WordId) -> f32 {
        let Some(h) = self.heard.word(id) else {
            return 0.5;
        };
        let mut kept = self.wild.loudness.borrow_mut();
        if let Some(&(sound, loud)) = kept.get(&id) {
            if sound == h.sound {
                return loud;
            }
        }
        let levels = self.kit_raw_levels();
        let (a, b) = h.sound;
        let peak = (0..=8)
            .map(|i| levels.at(a + (b - a) * i as f64 / 8.0))
            .fold(0.0f32, f32::max)
            .clamp(0.0, 1.0);
        let oldest = levels.newest_at - levels.levels.len().saturating_sub(1) as f64 / FRAME_HZ as f64;
        if !levels.levels.is_empty() && a >= oldest && b <= levels.newest_at {
            if kept.len() >= LOUDNESS_KEEP {
                kept.retain(|id, _| self.heard.word(*id).is_some());
            }
            kept.insert(id, (h.sound, peak));
        }
        peak
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
    /// the caret, slackening over 3 s of silence. A line whose `look` has
    /// gutters exhales after 3 s of silence. `drift_clip`, for an older line, is the height of the
    /// line's own box: with `lattice-drift` its lattice then drifts, clipped
    /// to that height so it never reaches the next line. `None` (the newest
    /// line) never drifts.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lattice_line(
        &self,
        key: String,
        words: &[Word],
        style: &TextStyle,
        size: Size,
        look: &LatticeLook,
        caret: Option<f32>,
        drift_clip: Option<f32>,
    ) -> Element<Msg> {
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        let nodes = anchors(&self.measure, &texts, style, None);
        let timed = self.lattice_words(words);
        let mut look = look.clone();
        look.plucks = lattice_plucks(&timed, self.heard.onsets()).into();
        let floor = self.lattice_floor();
        let mut voice = self.wild.voice.borrow_mut();
        // Every line feeds the voice, so a line drawn before the newest one
        // already hears this view's levels; frames fed before are skipped.
        voice.feed(&self.kit_raw_levels(), floor);
        look.quiet = lattice_quiet(&look, &timed, voice.voiced);
        if let (Some(caret_x), Some(newest)) = (caret, timed.len().checked_sub(1)) {
            look.level = Some(voice.level.clone());
            look.level_floor = floor;
            look.live = Some(LiveStrand {
                from_node: newest,
                caret_x,
                since: voice.voiced.max(timed[newest].at),
            });
        }
        drop(voice);
        let reduced = self.wild_reduced();
        let clip = drift_clip.filter(|_| self.lattice_drifts());
        look.drift = clip.map(|_| LatticeDrift { seed: drift_seed(&key), ..LatticeDrift::default() });
        let view = look.view(key.clone(), size, timed.into(), nodes.into(), reduced);
        match clip {
            Some(h) => Element::layer(format!("{key}-clip"))
                .size(size.w, h.min(size.h))
                .clip()
                .child(view),
            None => view,
        }
    }

    /// The words of `words` line by line as `style` wraps them at `wrap`,
    /// as a live text of the same words lays them out.
    pub(crate) fn wrap_lines(&self, words: &[Word], style: &TextStyle, wrap: f32) -> Vec<Vec<Word>> {
        let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let shaped = self.measure.layout(&text, style, Some(wrap));
        let line_of = |gi: usize| shaped.lines.iter().position(|l| l.glyphs.contains(&gi));
        let mut out: Vec<(usize, Vec<Word>)> = Vec::new();
        let mut start = 0usize;
        for w in words {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let line = shaped
                .glyphs
                .iter()
                .position(|g| range.contains(&(g.cluster as usize)))
                .and_then(line_of);
            match (out.last_mut(), line) {
                (Some((l, ws)), Some(n)) if *l == n => ws.push(w.clone()),
                (Some((_, ws)), None) => ws.push(w.clone()),
                (_, n) => out.push((n.unwrap_or(0), vec![w.clone()])),
            }
        }
        out.into_iter().map(|(_, ws)| ws).collect()
    }

    /// LT-07: the kinetic lattice under a wrapped transcript of `words` set
    /// in `style` at `wrap`, for a look with no lattice of its own (mission,
    /// tron, swiss), as one layer laid from the text's top left: one lattice
    /// per line, each in its gutter (`set`), labels on the newest only. While
    /// the popup listens a live strand runs from the newest word to where the
    /// next word will start. Older lines drift with `lattice-drift`. It fades
    /// out while a cancel rewinds the words or the text is handed off.
    /// `None` without words.
    pub(crate) fn lattice_lines(
        &self,
        key: &str,
        words: &[Word],
        style: &TextStyle,
        wrap: f32,
        set: &LatticeSet,
    ) -> Option<Element<Msg>> {
        if words.is_empty() {
            return None;
        }
        let lines = self.wrap_lines(words, style, wrap);
        let lh = style.line_height;
        let listening = self.files.recording && !self.files.transcribing && self.ending().is_none();
        let box_w = wrap + 4.0 * style.size;
        let mut layer = Element::layer(format!("{key}-lattice"))
            .size(box_w, lines.len() as f32 * lh + set.gutter);
        for (i, line) in lines.iter().enumerate() {
            let newest = i + 1 == lines.len();
            let text = line.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            let shaped = self.measure.layout(&text, style, None);
            let baseline = shaped.lines.first().map_or(style.size, |l| l.baseline);
            let g = line_gutter(style, baseline, newest, set.gutter);
            let label = set.label.filter(|_| newest);
            let mut look = set.preset.look(set.color, set.accent, set.scale, label);
            look.gutters = vec![g];
            let look = fit_look(look, &g);
            // The live strand ends a space past the line's ink, where the
            // next word will start.
            let caret = (newest && listening).then_some(shaped.size.w + 0.3 * style.size);
            let h = if newest { lh + set.gutter } else { lh };
            layer = layer.child(kit::at(
                self.lattice_line(
                    format!("{key}-{}", line[0].id),
                    line,
                    style,
                    Size::new(box_w, h),
                    &look,
                    caret,
                    (!newest).then_some(lh),
                ),
                0.0,
                i as f32 * lh,
            ));
        }
        // The lattice leaves with the words: a rewind or a handoff.
        let leaving = self.rewinding() || self.delivered().is_some();
        Some(match (leaving, self.wild_reduced()) {
            (false, _) => layer.opacity(set.opacity),
            (true, true) => layer.opacity(0.0).motion(Prop::Opacity, Motion::Snap),
            (true, false) => layer.opacity(0.0).motion(Prop::Opacity, Spring::critical_to_98(0.3)),
        })
    }

    /// Whether older lines' lattices drift: `lattice-drift` is on, the
    /// popup is not closing, and motion is not reduced.
    pub(crate) fn lattice_drifts(&self) -> bool {
        self.look.on(LATTICE_DRIFT) && !self.closing && !self.wild_reduced()
    }

    /// The depth stack of `lines` (oldest first, each with its width) in a
    /// box `w` wide and `rows` lines tall: the newest line flat at the
    /// bottom, the older ones receding on glass sheets above it. `newest`
    /// wraps the newest line's live text, for the look's typographic
    /// channels (`Popup::typo_words`). `hold` keeps the line just behind
    /// the newest flat for a moment ([`HOLD_SECS`], [`GLIDE_SECS`]).
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
        newest: &dyn Fn(Element<Msg>) -> Element<Msg>,
        hold: Option<&Hold>,
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
                (depth_stack_key(l.0.first().map_or(0, |w| w.id)), l)
            })
            .collect();
        let content = |line: &&(Vec<Word>, f32), d: usize, p: &Place| {
            let (words, width) = *line;
            let lw = width.ceil() + 2.0;
            let live = Element::live_text("words", words)
                .text_style(style)
                .color(text)
                .text_halo(glow.fade(0.45), 7.0 * k)
                .text_blur(p.blur)
                .motion(Prop::TextFx, stack.motion(d, reduced))
                .size(lw, line_h)
                // A word lands in place: the sheet of a new line rises 3 px
                // already, and a rise of its own on top breaks the reading
                // line's envelope (6ofh).
                .word_enter(Transition::fade().opacity(0.7))
                .word_change(Transition::fade().opacity(0.7));
            kit::at(
                if d == 0 { newest(live) } else { live },
                ((sheet.w - lw) / 2.0).max(0.0),
                4.0 * k,
            )
        };
        let h = rows.max(1) as f32 * line_h;
        let x = (w - sheet.w) / 2.0;
        kit::at(
            stack.view_at(
                "depth-stack",
                Size::new(sheet.w, h),
                sheet,
                &keyed,
                content,
                reduced,
                hold,
                self.extra.now,
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

/// How long a line that leaves the reading place as the box grows stays
/// flat where it was read before it moves back. Long enough for a centred
/// line to stop gliding along its row, so it is seen at rest there and not
/// on the turned sheet it moves back to (6ofh).
pub(crate) const HOLD_SECS: f64 = 0.35;

/// How long a line that leaves the reading place in a full box glides up
/// flat before it turns back, so it leaves the new line's row before it
/// turns (6ofh).
pub(crate) const GLIDE_SECS: f64 = 0.05;

/// The key of a depth-stack sheet whose line starts with word `first`.
pub(crate) fn depth_stack_key(first: WordId) -> vo::scene::Key {
    vo::scene::Key::from(format!("sheet{first}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The popup, reporting the first word's loudness against the newest
    /// level's time after every message.
    struct Loudness(Popup, std::rc::Rc<RefCell<Vec<(f64, f32)>>>);

    impl vo::App for Loudness {
        type Msg = Msg;

        fn init(&mut self, cx: &mut Cx<Msg>) {
            self.0.init(cx);
        }

        fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
            self.0.update(msg, cx);
            let first = (0..8).map(|i| i as WordId).find(|&id| self.0.heard.word(id).is_some());
            if let Some(id) = first {
                let at = self.0.kit_raw_levels().newest_at;
                self.1.borrow_mut().push((at, self.0.word_loudness(id)));
            }
        }

        fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
            self.0.view(tokens)
        }

        fn surfaces(&self, tokens: &vo::Tokens) -> Vec<vo::SurfaceView<Msg>> {
            self.0.surfaces(tokens)
        }

        fn main_surface(&self) -> Option<(Size, vo::Placement)> {
            self.0.main_surface()
        }
    }

    /// LT-12: a word keeps its loudness after its sound scrolls out of the
    /// level history, so its strand does not thin seconds after it landed
    /// (seen under reduced motion as the first two strands changing 3.6 and
    /// 3.9 s into the long-silence moment).
    #[test]
    fn a_word_keeps_its_loudness_after_its_sound_scrolls_away() {
        let theme = PlasmaTheme::default();
        let fonts = load_fonts(&theme).unwrap();
        let mut look = Look::new(&OsdConfig::default(), &theme, String::new(), &fonts);
        look.theme = OsdTheme::Scope;
        let s = super::moments::scenario(&look, super::moments::Moment::LongSilence);
        let seen = std::rc::Rc::new(RefCell::new(Vec::new()));
        vo::exit_check(Loudness(Popup::scripted(look, &fonts), seen.clone()), &s, 10.0, Options::default());
        let seen = seen.borrow();
        // From when the first word's sound is fully heard, to well past the
        // few seconds of level history.
        let settled: Vec<f32> = seen.iter().filter(|(at, _)| (1.0..6.0).contains(at)).map(|&(_, l)| l).collect();
        assert!(seen.iter().any(|(at, _)| *at > 5.5), "the silence runs past the history");
        assert!(settled.len() > 50, "{} samples", settled.len());
        assert!(settled[0] > 0.05, "the word was heard: {}", settled[0]);
        assert!(settled.iter().all(|&l| l == settled[0]), "loudness moved: {settled:?}");
    }

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
    fn the_exhale_counts_from_the_voice_or_the_newest_word_and_needs_gutters() {
        use vo::scene::lattice::LineGutter;
        let words = [LatticeWord::new(0.0, 0.5, 1.0), LatticeWord::new(0.2, 0.5, 2.5)];
        let bare = LatticeLook::default();
        assert_eq!(lattice_quiet(&bare, &words, 4.0), None, "no gutters, no exhale");
        let gutters = vec![LineGutter { top: 26.0, height: 14.0, newest: true, descent: 6.0 }];
        let look = LatticeLook { gutters, ..LatticeLook::default() };
        assert_eq!(lattice_quiet(&look, &words, 4.0), Some(Quiet::since(4.0)));
        assert_eq!(lattice_quiet(&look, &words, f64::NEG_INFINITY), Some(Quiet::since(2.5)));
        assert_eq!(lattice_quiet(&look, &[], 4.0), None);
        // Quiet from 4 s: no exhale until 7 s, full by 8.2 s, gone once the voice is back.
        let l = LatticeLook { quiet: lattice_quiet(&look, &words, 4.0), ..look.clone() };
        assert_eq!(l.exhale(6.9, false), 1.0);
        assert_eq!(l.exhale(8.3, false), 1.3);
        let back = LatticeLook { quiet: lattice_quiet(&look, &words, 8.3), ..look };
        assert_eq!(back.exhale(8.3, false), 1.0, "snaps back when the voice returns");
    }

    #[test]
    fn the_newest_gutter_reaches_under_the_box_and_older_ones_sit_between_lines() {
        let style = TextStyle::new(0, 24.0, 30.0);
        let newest = line_gutter(&style, 22.0, true, 10.0);
        assert!(newest.newest);
        assert_eq!(newest.top, 22.0 + 6.0);
        assert_eq!(newest.top + newest.height, 30.0 + 10.0, "ends `gutter` under the box");
        let older = line_gutter(&style, 22.0, false, 10.0);
        assert!(!older.newest);
        // From the descenders to the next line's ascenders.
        assert!((older.top + older.height - (30.0 + 22.0 - 24.0 * ASCENT)).abs() < 1e-4);
        // Lines set tighter than their type leave no gutter, not a negative one.
        let tight = line_gutter(&TextStyle::new(0, 38.4, 30.0), 30.0, false, 10.0);
        assert_eq!(tight.height, 0.0);
    }

    #[test]
    fn a_look_fits_its_gutter() {
        let label = Some(TextStyle::new(0, 8.0, 10.0));
        let look = LatticeStyle::Blueprint.look(Color::WHITE, Color::hex(0xff7a1a, 1.0), 1.0, label);
        // Room for the label under a shortened drop line.
        let g = LineGutter { top: 30.0, height: 30.0, newest: true, descent: 6.0 };
        let fit = fit_look(look.clone(), &g);
        assert!(fit.label.is_some());
        let label_bottom = g.top + fit.drop + fit.stem + 1.0 + 10.0;
        assert!(label_bottom <= g.top + g.height + 1.0, "label ends at {label_bottom}");
        // No room: no label, and the drop line stays in the gutter.
        let g = LineGutter { height: 12.0, ..g };
        let fit = fit_look(look.clone(), &g);
        assert_eq!(fit.label, None);
        assert!(g.top + fit.drop + fit.stem <= g.top + g.height);
        // An older line: fainter, no accent, no label, its marks in half the gutter.
        let g = LineGutter { top: 30.0, height: 8.0, newest: false, descent: 6.0 };
        let fit = fit_look(look.clone(), &g);
        assert_eq!(fit.label, None);
        assert_eq!(fit.accent, fit.color);
        assert!(fit.color.a < look.color.a);
        assert!(fit.stem <= 3.0 && fit.tick <= 3.0);
    }

    #[test]
    fn drift_seeds_differ_per_line() {
        assert_ne!(drift_seed("lattice-1"), drift_seed("lattice-2"));
    }

    #[test]
    fn a_new_recording_starts_the_voice_again() {
        let mut v = LatticeVoice::default();
        v.feed(&levels(&[0.6; 10], 9.0), 0.1);
        v.feed(&levels(&[0.03; 10], 1.0), 0.1);
        assert_eq!(v.voiced, f64::NEG_INFINITY);
    }
}
