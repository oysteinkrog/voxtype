//! EG-16 Digit lean: the words-per-minute counter leans its digits forward
//! with the pace, up to 12 degrees, and stands them back up in a pause.

use super::*;

use vo::scene::flow::letter_motion::digit_lean;

/// A word at least this often still counts as talking, in seconds.
const TALKING_GAP: f64 = 0.45;

pub(super) fn exhibit() -> Exhibit {
    Exhibit::new("EG-16", "Digit lean", OsdTheme::Swissfi)
        .feature(super::wild_motion::DIGIT_LEAN)
        .view(view)
        .moments(&[Moment::Dictation, Moment::Pause])
}

fn view(p: &Popup, _: &vo::Tokens) -> Element<Msg> {
    let size = p.look.surface();
    let t = p.extra.now;
    let reduced = p.look.on("reduced-motion") || kit::reduced();
    let dim = Color::rgba(0.55, 0.6, 0.67, 1.0);
    let paper = Color::rgba(0.93, 0.94, 0.96, 1.0);
    let pad = 24.0;
    let big = (0.32 * size.h).clamp(40.0, 96.0);
    // The pause runs from the last word, so the lean is a function of the
    // time and the words' arrivals alone.
    let last = p.tally.arrived.back().map_or(f64::NEG_INFINITY, |a| a.1);
    let pause = (t - last - TALKING_GAP).max(0.0);
    let pace = p.tally.recent_wpm(t);
    let lean = digit_lean(pace, pause, reduced);
    let counter = Element::label("wpm", p.tally.wpm(t).to_string())
        .text_style(p.look.mono(big))
        .color(paper)
        .glyph_lean(lean);
    let caption = Element::label("cap", format!("WORDS PER MINUTE  /  LEAN {lean:.1} DEG"))
        .text_style(p.look.mono(10.0))
        .color(dim);
    Element::panel("digit-lean")
        .size(size.w, size.h)
        .radius(10.0)
        .fill(Color::rgba(0.04, 0.055, 0.075, 0.97))
        .child(super::kit::at(caption, pad, 14.0))
        .child(super::kit::at(counter, pad, 0.14 * size.h))
        .signals(p.signals.clone())
}
