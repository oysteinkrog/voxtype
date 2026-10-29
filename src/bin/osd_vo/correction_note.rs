//! A spoken correction in any look (COR-3).
//!
//! While the correction key is held, the live words dim and the look's
//! status row says "Correcting" with the respoken words as they arrive.
//! When the hold is placed, the row shows the words it replaced and the new
//! ones for [`NOTE_SECS`]; when it was dropped, it says why. The words in
//! the look are already the corrected ones: the daemon makes the correction
//! in the preview. A dictation without corrections looks as before.

use super::*;

use voxtype::osd::correction::CorrectionView;

/// How long the outcome of a correction stays in the status row.
pub(crate) const NOTE_SECS: f64 = 1.6;

/// The live words' opacity while the key is held.
const DIMMED: f32 = 0.35;

/// Which outcome the status row shows, and since when.
#[derive(Debug, Default)]
pub(crate) struct State {
    note: Option<(u32, f64)>,
}

impl State {
    /// `preview.json` changed at `t`. Returns how long until the popup
    /// should be drawn again, when a new outcome arrived.
    pub(crate) fn files(&mut self, view: Option<&CorrectionView>, t: f64) -> Option<f64> {
        let id = view?.note.as_ref()?.id;
        if self.note.is_some_and(|(seen, _)| seen == id) {
            return None;
        }
        self.note = Some((id, t));
        Some(NOTE_SECS)
    }

    fn note_shown(&self, now: f64) -> bool {
        self.note.is_some_and(|(_, at)| now - at < NOTE_SECS)
    }
}

/// The look's live words, dimmed while the key is held.
pub(crate) fn dim(look: Element<Msg>, view: Option<&CorrectionView>) -> Element<Msg> {
    if view.is_some_and(|v| v.hearing.is_some()) {
        look.map_live_texts(&mut |e| e.opacity(DIMMED))
    } else {
        look
    }
}

/// The status row's state and detail for the correction, when it has one
/// to show.
pub(crate) fn status(
    view: Option<&CorrectionView>,
    state: &State,
    now: f64,
) -> Option<(String, String)> {
    let view = view?;
    if let Some(heard) = &view.hearing {
        let heard = if heard.is_empty() {
            "say the right words".to_string()
        } else {
            heard.clone()
        };
        return Some(("Correcting".into(), format!("  ·  {heard}")));
    }
    if view.pending {
        return Some(("Correcting".into(), "  ·  placing it".into()));
    }
    let note = view.note.as_ref().filter(|_| state.note_shown(now))?;
    Some(match &note.dropped {
        None => (
            "Corrected".into(),
            format!("  ·  {} → {}", note.from, note.to),
        ),
        Some(why) => ("Not changed".into(), format!("  ·  {why}: {}", note.to)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxtype::osd::correction::CorrectionNote;

    fn note(id: u32, dropped: Option<&str>) -> CorrectionView {
        CorrectionView {
            hearing: None,
            pending: false,
            note: Some(CorrectionNote {
                id,
                from: "franken term".into(),
                to: "FrankenTerm".into(),
                dropped: dropped.map(str::to_string),
            }),
        }
    }

    #[test]
    fn a_new_outcome_shows_for_its_time_once() {
        let mut s = State::default();
        assert_eq!(s.files(None, 1.0), None);
        assert_eq!(s.files(Some(&note(1, None)), 2.0), Some(NOTE_SECS));
        assert_eq!(s.files(Some(&note(1, None)), 2.5), None);
        assert!(s.note_shown(2.0 + NOTE_SECS - 0.1));
        assert!(!s.note_shown(2.0 + NOTE_SECS));
        assert_eq!(s.files(Some(&note(2, None)), 5.0), Some(NOTE_SECS));
    }

    #[test]
    fn the_status_row_follows_the_hold() {
        let mut s = State::default();
        let hold = |h: &str| CorrectionView {
            hearing: Some(h.into()),
            pending: false,
            note: None,
        };
        assert_eq!(status(None, &s, 0.0), None);
        assert_eq!(
            status(Some(&hold("")), &s, 0.0),
            Some(("Correcting".into(), "  ·  say the right words".into()))
        );
        assert_eq!(
            status(Some(&hold("FrankenTerm")), &s, 0.0).unwrap().1,
            "  ·  FrankenTerm"
        );
        let placed = note(1, None);
        s.files(Some(&placed), 1.0);
        assert_eq!(
            status(Some(&placed), &s, 1.5),
            Some(("Corrected".into(), "  ·  franken term → FrankenTerm".into()))
        );
        assert_eq!(status(Some(&placed), &s, 1.0 + NOTE_SECS), None);
        let dropped = note(2, Some("no match"));
        s.files(Some(&dropped), 3.0);
        assert_eq!(
            status(Some(&dropped), &s, 3.1),
            Some(("Not changed".into(), "  ·  no match: FrankenTerm".into()))
        );
    }
}
