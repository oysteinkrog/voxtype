//! The end of a dictation: how long the popup stays (MO-01), what the daemon
//! says it delivered (MO-02), the text leaving for the app (MO-42) and the
//! receipt left behind (MO-43).
//!
//! The daemon writes `result.json` after typing or pasting has finished, and
//! `outcome` just after it, so the popup learns of a delivery once it is
//! complete. The hand-off replays it at the rate the daemon reports (its
//! characters over `output_ms`), squeezed into `HANDOFF_MAX_SECS` for long
//! text. The receipt says what the daemon reports and nothing more: typed,
//! pasted or copied, and whether Enter was sent.

use std::sync::{Arc, Mutex};

use vo::scene::canvas::Painter;
use vo::scene::display::{GlyphInstance, Paint, ShapeInstance, NO_WORD};
use vo::scene::geom::{Affine, Point, RRect};
use vo::scene::path::{Contour, StrokeStyle};
use vo::scene::text::ShapedText;
use voxtype::osd::outcome::Delivery;

use super::*;

/// A normal end stays this long, so a look can show it. A new dictation cuts it short.
pub(crate) const DONE_LINGER_SECS: f64 = 0.4;

/// "No speech" stays this long, so the look can type it out and it can be read.
pub(crate) const EMPTY_LINGER_SECS: f64 = 1.0;

/// A typed hand-off never takes longer than this; faster typing plays at its own rate.
const HANDOFF_MAX_SECS: f64 = 0.7;
/// The typing rate's bounds, in seconds per character.
const CHAR_SECS: (f64, f64) = (0.006, 0.045);
/// How long one letter takes to shrink to its baseline.
const LETTER_SECS: f64 = 0.12;
/// Paste: lines collapse into the reading line, squash to a hairline, then stamp.
const COLLAPSE_SECS: f64 = 0.16;
const SQUASH_SECS: f64 = 0.12;
const STAMP_SECS: f64 = 0.08;
/// The return tick after the text, when Enter was sent.
const TICK_SECS: f64 = 0.18;
/// The receipt folds in over this and then stays until the popup closes.
const RECEIPT_SECS: f64 = 0.22;
/// Reduced motion: the text fades over this instead.
const FADE_SECS: f64 = 0.12;

/// How the text reached the app, as far as the daemon's driver name tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// Typed key by key (wtype, dotool, ydotool and the like).
    Typed,
    /// Put on the clipboard and pasted.
    Pasted,
    /// Only put on the clipboard.
    Copied,
}

impl Via {
    pub(crate) fn of(method: &str) -> Via {
        match method {
            "paste" => Via::Pasted,
            "clipboard" | "wl-copy" | "xclip" => Via::Copied,
            _ => Via::Typed,
        }
    }

    /// The status line's word for it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Via::Typed => "Typed",
            Via::Pasted => "Pasted",
            Via::Copied => "Copied",
        }
    }

    pub(crate) fn verb(self) -> &'static str {
        match self {
            Via::Typed => "typed",
            Via::Pasted => "pasted",
            Via::Copied => "copied",
        }
    }
}

/// A delivery the daemon reported, as the popup saw it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Delivered {
    pub via: Via,
    pub enter: bool,
    pub chars: usize,
    /// How long the typing or pasting took, in seconds.
    pub output_secs: f64,
    /// From the end of the recording to the text in the app, when known.
    pub latency_ms: Option<u64>,
    /// The app the text went to, when the daemon named it during the dictation.
    pub app: Option<String>,
    /// Scene time the popup saw the report.
    pub at: f64,
    /// Reduced motion: the text fades and the marks stand still.
    pub reduced: bool,
}

impl Delivered {
    pub(crate) fn new(d: &Delivery, app: Option<String>, at: f64) -> Delivered {
        Delivered {
            via: Via::of(&d.method),
            enter: d.enter,
            chars: d.chars,
            output_secs: d.output_ms as f64 / 1000.0,
            latency_ms: d.latency_ms,
            app,
            at,
            reduced: kit::reduced(),
        }
    }

    /// The receipt's words: "typed into konsole, Enter".
    pub(crate) fn receipt(&self) -> String {
        let mut s = self.via.verb().to_string();
        if let Some(app) = &self.app {
            let short = app.rsplit('.').next().unwrap_or(app);
            s.push_str(if self.via == Via::Copied {
                " for "
            } else {
                " into "
            });
            s.push_str(short);
        }
        if self.enter {
            s.push_str(", Enter");
        }
        s
    }

    /// Seconds per letter for `letters` letters of a typed hand-off.
    fn char_secs(&self, letters: usize) -> f64 {
        let n = letters.max(1) as f64;
        let real = (self.output_secs / self.chars.max(1) as f64).clamp(CHAR_SECS.0, CHAR_SECS.1);
        real.min((HANDOFF_MAX_SECS - LETTER_SECS) / n)
    }

    /// When the text has left, `letters` letters on `lines` lines, relative to `at`.
    fn text_gone(&self, letters: usize) -> f64 {
        if self.reduced {
            return FADE_SECS;
        }
        match self.via {
            Via::Typed => letters as f64 * self.char_secs(letters) + LETTER_SECS,
            Via::Pasted | Via::Copied => COLLAPSE_SECS + SQUASH_SECS + STAMP_SECS,
        }
    }

    /// When the hand-off and the receipt's fold are complete, relative to `at`.
    fn settled(&self, letters: usize) -> f64 {
        let tick = if self.enter { TICK_SECS } else { 0.0 };
        self.text_gone(letters) + tick + RECEIPT_SECS
    }
}

/// What the end of a dictation keeps.
#[derive(Default)]
pub(crate) struct State {
    /// The delivery, once the daemon reported it.
    pub delivered: Option<Delivered>,
    /// The newest app name the daemon gave during the dictation.
    pub app: Option<String>,
    /// The remaining-audio ruler (MO-41).
    pub ruler: ruler::Ruler,
    /// The hand-off's drawing for the text it was made for.
    cache: Mutex<Option<(String, f32, Arc<Handoff>)>>,
}

impl Popup {
    /// How long the popup stays after the dictation ended.
    pub(crate) fn linger_secs(&self) -> f64 {
        match self.ending() {
            Some(Ending::Cancelled | Ending::Failed) => CANCEL_LINGER_SECS,
            // "No speech" is typed out on the reading line, so it needs longer to read.
            Some(Ending::Empty) => EMPTY_LINGER_SECS,
            Some(Ending::Done) => {
                let handoff = self.life.delivered.as_ref().map_or(0.0, |d| {
                    let since = d.at - self.ended_at.unwrap_or(d.at);
                    since + d.settled(self.letters()) + 0.15
                });
                handoff.max(DONE_LINGER_SECS)
            }
            Some(Ending::Stopped) | None => 0.0,
        }
    }

    /// The word a look's status shows for a finished dictation: what the daemon
    /// reported (Typed, Pasted or Copied), or Done before or without a report.
    pub(crate) fn delivery_word(&self) -> &'static str {
        self.delivered().map_or("Done", |d| d.via.label())
    }

    /// [`Popup::delivery_word`] in capitals.
    pub(crate) fn delivery_word_caps(&self) -> &'static str {
        match self.delivered().map(|d| d.via) {
            Some(Via::Typed) => "TYPED",
            Some(Via::Pasted) => "PASTED",
            Some(Via::Copied) => "COPIED",
            None => "DONE",
        }
    }

    /// The delivery, while the dictation that made it has ended.
    pub(crate) fn delivered(&self) -> Option<&Delivered> {
        self.life.delivered.as_ref().filter(|_| self.ended())
    }

    /// Letters on screen, for the hand-off's pace.
    fn letters(&self) -> usize {
        self.shown
            .iter()
            .map(|w| w.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum()
    }

    /// Keeps the app name and notes a delivery, after the files changed at `t`.
    pub(crate) fn sync_lifecycle(&mut self, t: f64) {
        if self.busy() {
            // A new dictation: the old ending no longer applies.
            self.life.delivered = None;
        }
        if let Some(app) = self.kit_target() {
            if self.life.app.as_deref() != Some(app) {
                self.life.app = Some(app.to_string());
            }
        }
        self.sync_ruler(t);
        if self.life.delivered.is_none() && self.ending() == Some(Ending::Done) {
            if let Some(d) = &self.files.delivery {
                let mut d = Delivered::new(d, self.life.app.clone(), t);
                d.reduced |= self.look.on("reduced-motion");
                tracing::info!(via = ?d.via, enter = d.enter, chars = d.chars, "delivered");
                self.life.delivered = Some(d);
            }
        }
    }

    /// The look's transcript, or the text leaving for the app once the daemon reported
    /// the delivery (MO-42), ending in the receipt (MO-43). `style` and `width` must be
    /// the ones the look lays out `words` with, so the hand-off starts where the text is.
    /// `ink` is the text colour, `accent` the hairline's and the receipt's.
    pub(crate) fn handoff_or(
        &self,
        words: Element<Msg>,
        style: &TextStyle,
        width: Option<f32>,
        ink: Color,
        accent: Color,
    ) -> Element<Msg> {
        let Some(d) = self.delivered() else {
            return words;
        };
        let text = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            return words;
        }
        let key = width.unwrap_or(-1.0);
        let h = {
            let mut cache = self.life.cache.lock().unwrap_or_else(|e| e.into_inner());
            match cache.as_ref() {
                Some((t, w, h)) if *t == text && *w == key && h.delivered == *d => h.clone(),
                _ => {
                    let shaped = self.measure.layout(&text, style, width);
                    let small = TextStyle::new(MONO, (style.size * 0.62).max(10.0), style.size);
                    let receipt = self.measure.layout(&d.receipt(), &small, None);
                    let h = Arc::new(Handoff {
                        delivered: d.clone(),
                        shaped,
                        receipt,
                        size: style.size,
                        ink,
                        accent,
                        reduced: d.reduced,
                    });
                    *cache = Some((text, key, h.clone()));
                    h
                }
            }
        };
        let (w, hgt) = (h.shaped.size.w, h.shaped.size.h);
        let until = d.at + d.settled(self.letters()) + 0.05;
        let draw = h.clone();
        Element::canvas("handoff", until, move |t, rect, p| draw.paint(t, rect, p))
            .size(width.unwrap_or(w).max(w), hgt)
    }
}

/// One hand-off: the laid-out text, the receipt and how to move them.
pub(crate) struct Handoff {
    delivered: Delivered,
    shaped: ShapedText,
    receipt: ShapedText,
    /// The text's font size.
    size: f32,
    ink: Color,
    accent: Color,
    reduced: bool,
}

fn clamp01(x: f64) -> f32 {
    x.clamp(0.0, 1.0) as f32
}

fn ease_in(x: f32) -> f32 {
    x * x
}

fn ease_out(x: f32) -> f32 {
    1.0 - (1.0 - x) * (1.0 - x)
}

impl Handoff {
    fn letters(&self) -> usize {
        self.shaped.glyphs.len()
    }

    /// Draws the hand-off at scene time `t` in `rect`, the text's box.
    pub(crate) fn paint(&self, t: f64, rect: Rect, p: &mut Painter) {
        let d = &self.delivered;
        let s = t - d.at;
        let o = Point::new(rect.x, rect.y);
        let gone = d.text_gone(self.letters());
        if self.reduced {
            let a = 1.0 - clamp01(s / FADE_SECS);
            self.paint_text(o, a, |_| (0.0, 1.0, 1.0), p);
            // The marks come once the text has gone, without travel.
            let shown = if s >= FADE_SECS { 1.0 } else { 0.0 };
            self.paint_tick(o, rect.w, if d.enter { shown } else { 0.0 }, 0.0, p);
            if shown > 0.0 {
                self.paint_receipt(o, 1.0, p);
            }
            return;
        }
        match d.via {
            Via::Typed => {
                let per = d.char_secs(self.letters());
                self.paint_text(
                    o,
                    1.0,
                    |i| {
                        let k = ease_in(clamp01((s - i as f64 * per) / LETTER_SECS));
                        (0.0, 1.0 - k, 1.0 - k)
                    },
                    p,
                );
            }
            Via::Pasted | Via::Copied => self.paint_paste(o, s, p),
        }
        if d.enter {
            let k = clamp01((s - gone) / TICK_SECS);
            if k > 0.0 && k < 1.0 {
                self.paint_tick(
                    o,
                    rect.w,
                    1.0 - ease_in(k),
                    ease_out(k) * self.size * 0.5,
                    p,
                );
            }
        }
        let tick = if d.enter { TICK_SECS } else { 0.0 };
        let r = clamp01((s - gone - tick * 0.5) / RECEIPT_SECS);
        if r > 0.0 {
            self.paint_receipt(o, ease_out(r), p);
        }
    }

    /// Draws each glyph with `f(index) = (dy, scale, alpha)`, scaled toward its own
    /// baseline and centred on its advance.
    fn paint_text(
        &self,
        o: Point,
        alpha: f32,
        f: impl Fn(usize) -> (f32, f32, f32),
        p: &mut Painter,
    ) {
        for (i, g) in self.shaped.glyphs.iter().enumerate() {
            let (dy, k, a) = f(i);
            let a = a * alpha;
            if a <= 0.004 || k <= 0.01 {
                continue;
            }
            p.glyph(GlyphInstance {
                pos: Point::new(
                    o.x + g.pos.x + g.advance * (1.0 - k) * 0.5,
                    o.y + g.pos.y + dy,
                ),
                size: self.size * k,
                font: g.font,
                glyph: g.glyph,
                word: NO_WORD,
                color: self.ink.fade(a),
            });
        }
    }

    /// The reading line: the last line's baseline, left end and width.
    fn reading_line(&self) -> (f32, f32, f32) {
        self.shaped
            .lines
            .last()
            .map_or((self.size, 0.0, 0.0), |l| (l.baseline, l.x, l.width))
    }

    /// Paste: every line slides down into the reading line and fades as it lands, the
    /// line squashes into a bright 2 px hairline, and the hairline stamps down and goes.
    fn paint_paste(&self, o: Point, s: f64, p: &mut Painter) {
        let (base, _, _) = self.reading_line();
        let c = ease_out(clamp01(s / COLLAPSE_SECS));
        let q = clamp01((s - COLLAPSE_SECS) / SQUASH_SECS);
        let st = clamp01((s - COLLAPSE_SECS - SQUASH_SECS) / STAMP_SECS);
        let last = self.shaped.lines.len().saturating_sub(1);
        // The middle of the x-height, where the hairline forms.
        let mid = base - self.size * 0.3;
        let mut x0 = f32::MAX;
        let mut x1 = f32::MIN;
        for (li, line) in self.shaped.lines.iter().enumerate() {
            x0 = x0.min(line.x);
            x1 = x1.max(line.x + line.width);
            let dy = (base - line.baseline) * c;
            let fade = if li == last { 1.0 } else { 1.0 - c };
            let squash = 1.0 - ease_in(q);
            if fade * squash <= 0.004 {
                continue;
            }
            // Squash about the hairline: a layer scaled in y only.
            let sy = squash.max(0.02);
            let m = Affine {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: sy,
                e: 0.0,
                f: (o.y + mid) * (1.0 - sy),
            };
            p.layer(m, fade * (0.4 + 0.6 * squash), |p| {
                for g in &self.shaped.glyphs[line.glyphs.clone()] {
                    p.glyph(GlyphInstance {
                        pos: Point::new(o.x + g.pos.x, o.y + g.pos.y + dy),
                        size: self.size,
                        font: g.font,
                        glyph: g.glyph,
                        word: NO_WORD,
                        color: self.ink,
                    });
                }
            });
        }
        if q > 0.0 && x1 > x0 {
            let bright = ease_out(q) * (1.0 - st);
            let y = o.y + mid - 1.0 + st * 4.0;
            let w = (x1 - x0) * (1.0 - 0.15 * st);
            let x = o.x + x0 + (x1 - x0 - w) * 0.5;
            let hot = mix(self.accent, Color::rgba(1.0, 1.0, 1.0, 1.0), 0.5);
            let line = RRect::new(Rect::new(x, y, w, 2.0), 1.0);
            p.shape(ShapeInstance::rrect(
                line,
                Paint::glow(hot.fade(bright), 6.0, 0.6),
            ));
            p.shape(ShapeInstance::rrect(line, Paint::solid(hot.fade(bright))));
        }
    }

    /// The return-shaped tick after the text, inside a box `width` wide, `alpha` faded
    /// and moved `dy` down, the way Enter sends the line on.
    fn paint_tick(&self, o: Point, width: f32, alpha: f32, dy: f32, p: &mut Painter) {
        if alpha <= 0.004 {
            return;
        }
        let (base, x, w) = self.reading_line();
        let u = self.size * 0.42;
        let x = o.x + (x + w + self.size * 0.35).min(width - u - 2.0).max(0.0);
        let y = o.y + base - self.size * 0.12 + dy;
        let pts = [
            Point::new(x + u, y - u * 1.1),
            Point::new(x + u, y),
            Point::new(x, y),
        ];
        let head = [
            Point::new(x + u * 0.38, y - u * 0.38),
            Point::new(x, y),
            Point::new(x + u * 0.38, y + u * 0.38),
        ];
        let style = StrokeStyle::solid((self.size * 0.08).max(1.5), self.accent.fade(alpha));
        p.stroke(&Contour::polyline(&pts, false), &style);
        p.stroke(&Contour::polyline(&head, false), &style);
    }

    /// The receipt: one small bar per line, as long as the line, folds into a notch on
    /// the reading line, with what the daemon reported beside it.
    fn paint_receipt(&self, o: Point, k: f32, p: &mut Painter) {
        let (base, x0, _) = self.reading_line();
        let n = self.shaped.lines.len().max(1);
        let notch = self.size * 0.9;
        let widest = self
            .shaped
            .lines
            .iter()
            .map(|l| l.width)
            .fold(1.0f32, f32::max);
        let row = 2.0f32;
        let top = o.y + base - n.min(6) as f32 * row * 1.5;
        for (i, line) in self.shaped.lines.iter().rev().take(6).enumerate() {
            // From the line's own place and length to its miniature in the notch.
            // A paste has already pressed every line into the reading line.
            let from_y = match self.delivered.via {
                Via::Typed => line.baseline,
                Via::Pasted | Via::Copied => base,
            };
            let full = (line.x, from_y - self.size * 0.3, line.width);
            let mini = (
                x0,
                top - o.y + (n.min(6) - 1 - i) as f32 * row * 1.5,
                notch * line.width / widest,
            );
            let lerp = |a: f32, b: f32| a + (b - a) * k;
            let (x, y, w) = (
                lerp(full.0, mini.0),
                lerp(full.1, mini.1),
                lerp(full.2, mini.2),
            );
            let h = lerp(1.0, row);
            p.shape(ShapeInstance::rrect(
                RRect::new(Rect::new(o.x + x, o.y + y, w.max(1.0), h), 0.0),
                Paint::solid(self.accent.fade(0.35 + 0.45 * k)),
            ));
        }
        let lx = o.x + x0 + notch + self.size * 0.4;
        let a = k * k;
        let own = self.receipt.lines.first().map_or(0.0, |l| l.baseline);
        for g in &self.receipt.glyphs {
            p.glyph(GlyphInstance {
                pos: Point::new(lx + g.pos.x, o.y + base + g.pos.y - own),
                size: self.receipt_size(),
                font: g.font,
                glyph: g.glyph,
                word: NO_WORD,
                color: self.ink.fade(0.8 * a),
            });
        }
    }

    fn receipt_size(&self) -> f32 {
        (self.size * 0.62).max(10.0)
    }
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(method: &str, chars: usize, output_ms: u64, enter: bool) -> Delivered {
        Delivered::new(
            &Delivery {
                text: "x".repeat(chars),
                chars,
                method: method.into(),
                enter,
                output_ms,
                latency_ms: Some(500),
                audio_s: None,
            },
            Some("org.kde.konsole".into()),
            1.0,
        )
    }

    #[test]
    fn the_driver_says_how_the_text_went() {
        assert_eq!(Via::of("wtype"), Via::Typed);
        assert_eq!(Via::of("dotool"), Via::Typed);
        assert_eq!(Via::of("paste"), Via::Pasted);
        assert_eq!(Via::of("clipboard"), Via::Copied);
    }

    #[test]
    fn the_receipt_states_the_reported_result() {
        assert_eq!(
            delivery("wtype", 10, 50, true).receipt(),
            "typed into konsole, Enter"
        );
        assert_eq!(
            delivery("paste", 10, 50, false).receipt(),
            "pasted into konsole"
        );
        let mut d = delivery("wtype", 10, 50, false);
        d.app = None;
        assert_eq!(d.receipt(), "typed");
    }

    #[test]
    fn typing_plays_at_its_own_rate_and_long_text_is_squeezed() {
        // 20 ms a letter, 20 letters: 0.4 s at the real rate.
        let d = delivery("wtype", 20, 400, false);
        assert!((d.char_secs(20) - 0.02).abs() < 1e-9);
        // 400 letters at 20 ms would take 8 s: squeezed to the cap.
        let d = delivery("wtype", 400, 8000, false);
        let total = 400.0 * d.char_secs(400) + LETTER_SECS;
        assert!(total <= HANDOFF_MAX_SECS + 1e-9, "{total}");
    }

    #[test]
    fn enter_adds_the_tick_to_the_hand_off() {
        let a = delivery("paste", 30, 40, false).settled(30);
        let b = delivery("paste", 30, 40, true).settled(30);
        assert!((b - a - TICK_SECS).abs() < 1e-9);
    }
}
