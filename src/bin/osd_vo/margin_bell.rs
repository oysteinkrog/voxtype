//! The margin bell (MO-31), behind the motion feature `margin-bell`: when
//! the reading line is nearly full its letters tighten a little, up to
//! 3.5 % (inside the 5 % reading-line envelope), as a typewriter's bell
//! warns of the margin. When the line wraps the tightened words relax as
//! the new line starts. Type and phosphor also give the text a 3 px
//! carriage-return shift on the wrap, which settles in 250 ms.
//!
//! The tracking goes through the typographic channels
//! ([`Popup::typo_words`]), so it shows in the looks that use them (swiss,
//! type, swissfi). The shift is [`Popup::bell_shift`], an x offset for the
//! live text. Under reduced motion the tracking still tightens (a state
//! cue) but jumps, and there is no shift.

use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use vo_core::words::WordId;

pub(crate) const BELL: &str = "margin-bell";

/// The line starts to tighten when its ink reaches this share of the width,
/// and is tightest from `FULL`: a line wraps with about a word's room left.
const START: f32 = 0.70;
const FULL: f32 = 0.92;
/// Tightest tracking at a full line: 3.5 % under normal. Each word tightens
/// about its own middle, so more would open visible gaps between words.
const TIGHT: f32 = 0.035;
/// The carriage-return shift and how long it takes to settle.
const SHIFT_PX: f32 = 3.0;
const SHIFT_SECS: f64 = 0.25;

/// What the bell keeps between views.
#[derive(Default)]
pub(crate) struct State(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// The first word of the reading line at the last view.
    head: Option<WordId>,
    /// When the reading line last wrapped.
    wrap_at: Option<f64>,
}

/// The reading line of `words` laid out in `style` at `width`: the ids of
/// its words and how full it is, 0 to 1.
fn reading_line(p: &Popup, style: &TextStyle, width: f32) -> (Vec<WordId>, f32) {
    let mut joined = String::new();
    let mut starts = Vec::new();
    for w in &p.shown {
        if !joined.is_empty() {
            joined.push(' ');
        }
        starts.push((w.id, joined.len()));
        joined.push_str(&w.text);
    }
    let laid = p.measure.layout(&joined, style, Some(width));
    let Some(line) = laid.lines.last() else {
        return (Vec::new(), 0.0);
    };
    let Some(first) = laid.glyphs.get(line.glyphs.start) else {
        return (Vec::new(), 0.0);
    };
    let from = first.cluster as usize;
    let ids = starts
        .iter()
        .filter(|(_, at)| *at >= from)
        .map(|(id, _)| *id)
        .collect();
    (ids, (line.width / width.max(1.0)).clamp(0.0, 1.0))
}

impl Popup {
    pub(crate) fn bell_on(&self) -> bool {
        self.look.on(BELL)
    }

    /// Tracking factors for the words of the reading line, and notes a
    /// wrap. Empty when the bell is off or the line has room.
    pub(crate) fn bell_tracking(&self) -> HashMap<WordId, f32> {
        if !self.bell_on() {
            return HashMap::new();
        }
        let l = &self.look;
        let (ids, full) = reading_line(self, &l.text_style(), l.text_width());
        {
            let mut s = self.bell.0.lock().unwrap();
            let head = ids.first().copied();
            // A new reading line whose first word is the newest: a wrap.
            if head != s.head
                && head.is_some()
                && head == self.shown.last().map(|w| w.id)
                && s.head.is_some()
            {
                s.wrap_at = Some(self.extra.now);
            }
            s.head = head;
        }
        let k = ((full - START) / (FULL - START)).clamp(0.0, 1.0);
        let k = k * k * (3.0 - 2.0 * k);
        if k <= 0.0 {
            return HashMap::new();
        }
        ids.into_iter().map(|id| (id, 1.0 - TIGHT * k)).collect()
    }

    /// The carriage-return shift of the live text in px at this view: 3 px
    /// right at the wrap, eased back to 0 over 250 ms. 0 when the bell is
    /// off, under reduced motion, or between wraps.
    pub(crate) fn bell_shift(&self) -> f32 {
        if !self.bell_on() || self.look.on("reduced-motion") {
            return 0.0;
        }
        let _ = self.bell_tracking();
        let Some(at) = self.bell.0.lock().unwrap().wrap_at else {
            return 0.0;
        };
        let u = ((self.extra.now - at) / SHIFT_SECS).clamp(0.0, 1.0) as f32;
        SHIFT_PX * (1.0 - u) * (1.0 - u)
    }
}
