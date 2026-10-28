//! MO-63 the trace is the sentence, on scope: once a word comes, the
//! stretch of trace it came from freezes, shrinks and drops into a strip
//! along the foot of the graticule with a tick at its start.

use super::*;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("MO-63", "The trace is the sentence", OsdTheme::Scope)
        .feature(scope::SENTENCE_TRACE)
        .moments(&[Moment::Dictation, Moment::Pause, Moment::Cancel])
}
