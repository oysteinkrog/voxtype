//! `voxtype-osd-vo --entry-latency`: how long a look's word-entry animation
//! keeps a new word from being readable (LAT-1).
//!
//! Runs the scripted dictation (or `--moment NAME`) on the CPU, as
//! `--envelope` does, and follows each transcript word from the first frame
//! it is drawn in, at any ink. The word is readable from the first frame its
//! ink is at least 90 % of the most it has in its first 0.5 s, and settled
//! from the first frame after which its baseline holds within 1 px for
//! 0.1 s. A later reflow (a wrap, a scroll) moves a word that had already
//! settled, so it does not count. Prints the median, 90th percentile and
//! largest of each in milliseconds, and the word count.

use std::collections::HashMap;

use vo::scene::display::{DisplayList, ItemKind, NO_WORD};
use vo::scene::geom::Point;
use vo::{FontRegistry, Options};

use super::{default_motion, scenario, Look, Popup};

/// Transcript word ids count up from 0; status rows use hashed ids above this.
const TRANSCRIPT_IDS: u32 = 1 << 20;
/// The word's peak ink is taken over this long after it first shows.
const PEAK_WITHIN: f64 = 0.5;
/// A word is settled once its baseline holds this long.
const HOLD: f64 = 0.1;
const INK_SHARE: f32 = 0.9;
const BASELINE_PX: f32 = 1.0;

/// One word in one frame: the least ink of its glyphs and its mean baseline.
#[derive(Clone, Copy, Debug)]
struct Seen {
    ink: f32,
    y: f32,
}

/// Each transcript word's ink and baseline in `list`, in surface space.
fn words_in(list: &DisplayList) -> HashMap<u32, Seen> {
    let mut opacity = vec![1.0f32; list.layers.len()];
    for (i, l) in list.layers.iter().enumerate() {
        let parent = l.parent.map_or(1.0, |p| opacity[p as usize]);
        opacity[i] = parent * l.opacity;
    }
    let mut acc: HashMap<u32, (f32, f32, u32)> = HashMap::new();
    for (li, l) in list.layers.iter().enumerate() {
        for item in &list.items[l.items.start as usize..l.items.end as usize] {
            if item.kind != ItemKind::Glyphs {
                continue;
            }
            for gi in item.start..item.start + item.len {
                let g = &list.glyphs[gi as usize];
                if g.word == NO_WORD || g.word >= TRANSCRIPT_IDS {
                    continue;
                }
                let fx = list
                    .glyph_fx
                    .binary_search_by_key(&gi, |(i, _)| *i)
                    .ok()
                    .map(|k| list.glyph_fx[k].1)
                    .unwrap_or_default();
                let ink = g.color.a
                    * opacity[li]
                    * (1.0 - fx.dissolve.clamp(0.0, 1.0))
                    * (1.0 - fx.wipe.clamp(0.0, 1.0));
                let at = Point::new(g.pos.x + g.xform.offset.x, g.pos.y + g.xform.offset.y);
                let y = l.transform.apply(at).y;
                let e = acc.entry(g.word).or_insert((f32::INFINITY, 0.0, 0));
                e.0 = e.0.min(ink);
                e.1 += y;
                e.2 += 1;
            }
        }
    }
    acc.into_iter()
        .map(|(w, (ink, y, n))| (w, Seen { ink, y: y / n as f32 }))
        .collect()
}

/// The entry of each word that stayed drawn for `PEAK_WITHIN`: when its ink
/// first reached 90 % of its peak (readable), and when its baseline first held
/// within 1 px for `HOLD` (settled).
fn entry_times(frames: &[(f64, HashMap<u32, Seen>)]) -> (Vec<f64>, Vec<f64>) {
    let mut first: HashMap<u32, usize> = HashMap::new();
    for (i, (_, seen)) in frames.iter().enumerate() {
        for &w in seen.keys() {
            first.entry(w).or_insert(i);
        }
    }
    let (mut ready, mut settled) = (Vec::new(), Vec::new());
    for (&w, &i0) in &first {
        let t0 = frames[i0].0;
        let window: Vec<_> = frames[i0..]
            .iter()
            .take_while(|(t, _)| *t <= t0 + PEAK_WITHIN)
            .filter_map(|(t, s)| s.get(&w).map(|s| (t - t0, *s)))
            .collect();
        // Words that leave within the window (a cut, the end) are not counted.
        if window.last().is_none_or(|(t, _)| *t < PEAK_WITHIN - 0.05) {
            continue;
        }
        let peak = window.iter().map(|(_, s)| s.ink).fold(0.0, f32::max);
        if let Some((t, _)) = window.iter().find(|(_, s)| s.ink >= INK_SHARE * peak) {
            ready.push(*t);
        }
        let holds = |i: usize| {
            let (t, s) = window[i];
            window[i..]
                .iter()
                .take_while(|(u, _)| *u <= t + HOLD)
                .all(|(_, o)| (o.y - s.y).abs() <= BASELINE_PX)
        };
        if let Some(i) = (0..window.len()).find(|&i| holds(i)) {
            settled.push(window[i].0);
        }
    }
    ready.sort_by(f64::total_cmp);
    settled.sort_by(f64::total_cmp);
    (ready, settled)
}

pub(crate) fn run(look: Look, fonts: FontRegistry, fps: f64, moment: Option<&str>) -> anyhow::Result<()> {
    let scenario = match moment {
        Some(name) => {
            let m = super::moments::Moment::parse(name).ok_or_else(|| {
                anyhow::anyhow!("no moment {name:?}; the moments are {}", super::moments::Moment::names())
            })?;
            super::moments::scenario(&look, m)
        }
        None => scenario(&look, false),
    };
    let options = Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        ..Options::default()
    };
    let mut frames = Vec::new();
    vo::envelope::sample_scenario(Popup::new(look.clone(), &fonts), &scenario, fps, options, &mut |_, t, list| {
        frames.push((t, words_in(list)))
    });
    let (ready, settled) = entry_times(&frames);
    let ms = |v: &[f64], q: f64| {
        v.get(((v.len() as f64 - 1.0) * q).round() as usize)
            .map_or(f64::NAN, |t| t * 1000.0)
    };
    println!(
        "entry-latency: {:?} words={} readable median={:.0} p90={:.0} max={:.0} ms; settled median={:.0} p90={:.0} max={:.0} ms",
        look.theme,
        ready.len(),
        ms(&ready, 0.5),
        ms(&ready, 0.9),
        ms(&ready, 1.0),
        ms(&settled, 0.5),
        ms(&settled, 0.9),
        ms(&settled, 1.0),
    );
    Ok(())
}
