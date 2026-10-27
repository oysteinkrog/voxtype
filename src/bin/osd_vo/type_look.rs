//! The `type` look of the vo popup: the swiss popup's layout, colors and
//! behaviour, with the motion in the type and the lines rather than in
//! light. The frame draws itself on when the popup opens and is erased when
//! a dictation is cancelled; the clock and the word count roll their digits;
//! the icon turns from microphone to a stop square inside a spinner, and
//! the spinner closes into a ring round a check; the state label swaps by
//! sliding and changed numbers in the detail dip and settle; the rules open
//! from their middle; new words rise into place.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::*;

use std::sync::OnceLock;

use vo::scene::geom::Point;
use vo::scene::path::{icons, make, Morph, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Seconds for the frame to draw itself on.
const FRAME_SECS: f64 = 0.45;
/// Seconds for the icon to morph to its next shape.
const ICON_SECS: f64 = 0.30;
/// How far a new word rises into place, in logical pixels.
const WORD_RISE: f32 = 8.0;
/// How far the state label slides when it changes.
const LABEL_SLIDE: f32 = 7.0;
/// The digit size of the clock and the word count, against the swiss values.
const STAT_SCALE: f32 = 1.3;
/// The icon's box: room for the ring round the stop square and the check.
const ICON_BOX: f32 = 18.0;
/// Seconds for the ring to close once the final text lands.
const RING_SECS: f32 = 0.25;
/// Seconds for a rule to open from its middle.
const RULE_SECS: f64 = 0.35;

/// An element placed at `(x, y)` in its parent, out of the flow.
fn at<M>(e: Element<M>, x: f32, y: f32) -> Element<M> {
    e.layout(|s| {
        s.position = taffy::Position::Absolute;
        s.inset = taffy::Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        };
    })
}

impl Popup {
    pub(crate) fn type_view(&self) -> Element<Msg> {
        let root = Element::column("root").align_items(AlignItems::CENTER);
        if self.closing {
            return root;
        }
        root.child(self.type_panel())
    }

    fn type_panel(&self) -> Element<Msg> {
        let l = &self.look;
        let status = self.status();
        let h = l.panel_height(self.lines);
        // A cancelled dictation erases the frame; a finished one fades.
        let cancelled = self.extra.final_at.is_none();
        let exit = if cancelled {
            Transition::fade().scale(0.98)
        } else {
            Transition::fade().offset(0.0, -6.0)
        };
        Element::panel("panel")
            .width(l.width)
            .pad_edges(
                PAD_TOP + BORDER,
                PAD_X + BORDER,
                PAD_BOTTOM + BORDER,
                PAD_X + BORDER,
            )
            .radius(RADIUS)
            .fill(l.background)
            .presence(Transition::fade().offset(0.0, 6.0), exit)
            .child(self.type_frame(h, cancelled))
            .child(self.type_header(&status))
            .child(self.waveform(&status))
            .child(self.type_text())
            .child(
                self.type_rule("rule", l.width - 2.0 * (PAD_X + BORDER))
                    .margin_edges(RULE_MARGIN_TOP, 0.0, 0.0, 0.0),
            )
            .child(self.type_status(&status))
    }

    /// The panel's border as one line from the top middle round the panel,
    /// drawn on as the popup opens.
    fn type_frame(&self, h: f32, cancelled: bool) -> Element<Msg> {
        let l = &self.look;
        let border = Arc::new(Outline {
            contours: vec![make::panel(
                BORDER / 2.0,
                BORDER / 2.0,
                l.width - BORDER,
                h - BORDER,
                RADIUS,
            )],
        });
        let view = Rect::new(0.0, 0.0, l.width, h);
        let exit = if cancelled {
            // Erased from where it started.
            Transition::NONE.trim(1.0, 1.0)
        } else {
            Transition::fade()
        };
        at(
            Element::stroke_path("frame", border, view, StrokeStyle::solid(BORDER, l.border)),
            0.0,
            0.0,
        )
        .size(l.width, h)
        .motion(Prop::Trim, Spring::critical_to_98(FRAME_SECS))
        .presence(Transition::NONE.trim(0.0, 0.0), exit)
    }

    /// A hairline `w` wide that opens from its middle when it appears.
    fn type_rule(&self, key: &'static str, w: f32) -> Element<Msg> {
        let l = &self.look;
        let half = |side: &'static str, to: f32| {
            let line = Arc::new(Outline {
                contours: vec![make::underline(w / 2.0, to, 0.5)],
            });
            at(
                Element::stroke_path(
                    side,
                    line,
                    Rect::new(0.0, 0.0, w, 1.0),
                    StrokeStyle::solid(1.0, l.border),
                ),
                0.0,
                0.0,
            )
            .size(w, 1.0)
            .motion(Prop::Trim, Spring::critical_to_98(RULE_SECS))
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade())
        };
        Element::column(key)
            .size(w, 1.0)
            .child(half("left", 0.0))
            .child(half("right", w))
    }

    fn type_header(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        let caption = |key: &'static str, text: &str, px: f32, tracking: f32| {
            Element::label(key, text)
                .text_style(l.mono(px).tracking(tracking))
                .color(l.inactive)
        };
        let value = l.mono(l.value_px * STAT_SCALE);
        let stat = |key: &'static str, digits: Element<Msg>, name: &'static str| {
            Element::column(key)
                .gap(2.0)
                .align_items(AlignItems::END)
                .child(digits)
                .child(caption("caption", name, l.caption_small_px, 1.5))
        };
        let roll = Spring::critical_to_98(0.28);
        let clock = digit_roll("clock", &self.clock, value, l.foreground, roll);
        let words = digit_roll(
            "count",
            &self.word_count.to_string(),
            value,
            l.foreground,
            roll,
        );
        Element::row("header")
            .align_items(AlignItems::START)
            .child(self.type_icon(status))
            .child(
                caption("title", "VOICE DICTATION", l.caption_px, 2.0)
                    .margin_edges(0.0, 0.0, 0.0, 8.0),
            )
            .child(Element::row("spacer").grow(1.0))
            .child(stat("time", clock, "DURATION"))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .width(1.0)
                    .fill(l.border)
                    .margin_edges(0.0, 16.0, 0.0, 16.0)
                    .layout(|s| s.align_self = Some(AlignSelf::STRETCH)),
            )
            .child(stat("words", words, "WORDS"))
    }

    /// Microphone while recording; a stop square inside a spinning arc
    /// while the final text is made; then the arc closes into a ring and a
    /// check draws on inside it.
    fn type_icon(&self, status: &StatusLine) -> Element<Msg> {
        static MIC_TO_STOP: OnceLock<Arc<Morph>> = OnceLock::new();
        static CHECK: OnceLock<Arc<Outline>> = OnceLock::new();
        static RING: OnceLock<Arc<Outline>> = OnceLock::new();
        let l = &self.look;
        let icon_view = Rect::new(0.0, 0.0, 24.0, 24.0);
        let ring_view = Rect::new(0.0, 0.0, ICON_BOX, ICON_BOX);
        let ring = RING.get_or_init(|| {
            let c = ICON_BOX / 2.0;
            Arc::new(Outline {
                contours: vec![make::ring(Point::new(c, c), c - 1.0)],
            })
        });
        let slot = Element::column("icon").size(ICON_BOX, ICON_BOX);
        let done = self.extra.final_at.is_some();
        let stopped = !done && (status.finishing || self.extra.finish_at.is_some());
        let mut slot = slot;
        if done {
            let check = CHECK.get_or_init(|| Arc::new(Outline::new(&icons::check(), 0.05)));
            slot = slot
                .child(at(
                    Element::stroke_path(
                        "ring-closed",
                        ring.clone(),
                        ring_view,
                        StrokeStyle::solid(1.2, l.accent),
                    )
                    .size(ICON_BOX, ICON_BOX)
                    .motion(Prop::Trim, Spring::critical_to_98(RING_SECS as f64))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    0.0,
                    0.0,
                ))
                .child(at(
                    Element::stroke_path(
                        "check",
                        check.clone(),
                        icon_view,
                        StrokeStyle::solid(2.4, l.accent),
                    )
                    .size(10.0, 10.0)
                    // Slower than the ring, so the ring closes first.
                    .motion(Prop::Trim, Spring::critical_to_98(2.0 * ICON_SECS))
                    .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade()),
                    (ICON_BOX - 10.0) / 2.0,
                    (ICON_BOX - 10.0) / 2.0,
                ));
            return slot;
        }
        if stopped && !self.ended() {
            // The spinner: a dim track with a light head running round it.
            slot = slot.child(at(
                Element::stroke_path(
                    "ring-spin",
                    ring.clone(),
                    ring_view,
                    StrokeStyle::solid(1.2, l.border),
                )
                .size(ICON_BOX, ICON_BOX)
                .light_head(0.3, 0.8, l.accent, 0.0)
                .presence(Transition::fade().scale(0.6), Transition::fade()),
                0.0,
                0.0,
            ));
        }
        let morph = MIC_TO_STOP.get_or_init(|| {
            let mic = Outline::new(&icons::mic(), 0.05);
            let stop = Outline::new(&icons::stop(), 0.05);
            Arc::new(Morph::new(&mic, &stop, 64))
        });
        let side = if stopped { 9.0 } else { ICON };
        slot.child(at(
            Element::morph(
                "mic",
                morph.clone(),
                icon_view,
                StrokeStyle::solid(1.8, l.accent),
            )
            .size(side, side)
            .morph_to(if stopped { 1.0 } else { 0.0 })
            .motion(Prop::Morph, Spring::critical_to_98(ICON_SECS))
            .presence(Transition::fade(), Transition::fade().scale(0.6)),
            (ICON_BOX - side) / 2.0,
            (ICON_BOX - side) / 2.0,
        ))
    }

    fn type_text(&self) -> Element<Msg> {
        let l = &self.look;
        // Each new word rises into place, readable within 120 ms; a
        // revised word dips and settles.
        let words = Element::live_text("words", &self.shown)
            .text_style(l.text_style())
            .color(l.foreground)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().opacity(0.0).offset(0.0, WORD_RISE))
            .word_change(Transition::fade().opacity(0.35).offset(0.0, -4.0));
        Element::column("text")
            .height(self.lines as f32 * l.text_line)
            .margin_edges(0.0, 0.0, TEXT_MARGIN_BOTTOM, 0.0)
            .child(words)
    }

    fn type_status(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        let style = l.mono(l.status_px);
        let line = l.mono_line(l.status_px);
        // The state label slides up and out as the next one slides in.
        let state = status.state.as_str();
        let label_w = 0.62 * l.status_px * "FINISHING".len() as f32 + 4.0;
        let label = Element::layer("state")
            .clip()
            .size(label_w, line)
            .margin_edges(0.0, 0.0, 0.0, 10.0)
            .child(
                at(
                    Element::label(format!("state-{state}"), state)
                        .text_style(style)
                        .color(l.accent),
                    0.0,
                    0.0,
                )
                .presence(
                    Transition::fade().offset(0.0, LABEL_SLIDE),
                    Transition::fade().offset(0.0, -LABEL_SLIDE),
                ),
            );
        let dot = Element::shape("dot", ShapeKind::RRect)
            .size(DOT, DOT)
            .fill(if status.finishing {
                Color::TRANSPARENT
            } else {
                l.accent
            })
            .border(l.accent, 1.0);
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_MARGIN_TOP, 0.0, 0.0, 0.0)
            .child(dot)
            .child(label)
            .child(self.type_detail(status, style, line));
        if self.shows_progress(status) {
            // A hairline that fills with the progress.
            let progress = Progress {
                fraction: self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32),
                track: l.border,
                bar: l.accent,
                since: self.shown_at,
                period: 1.6,
            };
            row = row.child(
                Element::progress("progress", progress)
                    .height(1.0)
                    .grow(1.0)
                    .margin_edges(0.0, 0.0, 0.0, 16.0),
            );
        } else {
            row = row.child(Element::row("spacer").grow(1.0));
        }
        row.child(
            Element::label("mark", "//")
                .text_style(style)
                .color(l.inactive)
                .margin_edges(0.0, 0.0, 0.0, 12.0),
        )
    }

    /// The detail after the state label as live words. A word keeps its id
    /// while the state stays, so "4 s of audio left" turning to "3 s" dips
    /// and settles the number alone; a new state brings new words in.
    fn type_detail(&self, status: &StatusLine, style: TextStyle, line: f32) -> Element<Msg> {
        let l = &self.look;
        let base = status
            .state
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
            << 6;
        let words: Vec<Word> = status
            .detail
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| Word {
                id: base | i as u32 & 63,
                text: w.to_string(),
            })
            .collect();
        Element::live_text(format!("detail-{}", status.state), &words)
            .text_style(style)
            .color(l.inactive)
            .height(line)
            .margin_edges(0.0, 0.0, 0.0, 6.0)
            .shrink(1.0)
            .min_width(0.0)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(Transition::fade().offset(0.0, LABEL_SLIDE))
            .word_change(Transition::fade().opacity(0.2).offset(0.0, -LABEL_SLIDE))
            .presence(Transition::fade(), Transition::fade())
    }
}
