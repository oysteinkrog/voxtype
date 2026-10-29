//! The `type` look of the vo popup: the swiss popup's layout, colors and
//! behaviour, with the motion in the type and the lines rather than in
//! light. The frame draws itself on when the popup opens and is erased when
//! a dictation is cancelled; the clock and the word count roll their digits;
//! the status icon goes from record dot to stop square to spinner, and the
//! spinner closes into a ring round a check (an X on cancel); the state
//! label swaps by sliding and changed numbers in the detail dip and settle;
//! the rules open from their middle; new words rise into place, and a word
//! the engine is unsure of gets a wavy underline that straightens once it
//! is sure or the final pass starts.
//!
//! It shares the stop and final-text times with the showcase look
//! (`Extra::finish_at`, `Extra::final_at`).

use super::*;

use vo::scene::geom::Point;
use vo::scene::path::{make, Outline, StrokeStyle};
use vo::scene::roll::digit_roll;
use vo::scene::status_icon::{IconStyle, StatusIcon};
use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length};

/// Seconds for the frame to draw itself on.
const FRAME_SECS: f64 = 0.45;
/// How far a new word rises into place, in logical pixels.
const WORD_RISE: f32 = 8.0;
/// How far the state label slides when it changes.
const LABEL_SLIDE: f32 = 7.0;
/// The digit size of the clock and the word count, against the swiss values.
const STAT_SCALE: f32 = 1.3;
/// Room left of the word count for the notch its casts land in (MO-65).
const NOTCH_ROOM: f32 = 7.0;
/// The status icon's box.
const ICON_BOX: f32 = 18.0;
/// Seconds for the icon's done step (ring closes, then the check), so the
/// check is drawn before the popup closes, about half a second after the
/// final text.
const DONE_SECS: f32 = 0.45;
/// Below this confidence a word gets a wavy underline.
const DOUBT: f32 = 0.6;
/// The wavy underline: wave height and length, stroke width, in logical
/// pixels, and how far below the baseline it sits.
const WAVE_AMP: f32 = 1.6;
const WAVE_LEN: f32 = 6.0;
const WAVE_WIDTH: f32 = 1.2;
const WAVE_DROP: f32 = 2.0;
/// The underline's colour: amber, apart from the theme's accent.
const DOUBT_COLOR: u32 = 0xf5a524;

/// The y of the newest text line's centre from the surface top, with
/// `lines` text lines, for the reading-line placement. The panel grows
/// downward from the surface's top.
pub(crate) fn newest_line_centre(look: &Look, lines: u32) -> Option<f32> {
    Some(
        BORDER
            + PAD_TOP
            + Popup::type_header_height(look)
            + 2.0 * WAVE_MARGIN
            + swiss::WAVE_HEIGHT_PX as f32
            + (lines as f32 - 0.5) * look.text_line,
    )
}

/// What happened to the dictation, for the status icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconEvent {
    Record,
    Stop,
    Finish,
    Done,
    Cancel,
}

/// Notes the status icon's events from a change of the runtime files, after
/// `Extra::files` has noted the stop and the final text.
pub(crate) fn note_icon(x: &mut showcase::Extra, old: &Files, new: &Files, t: f64) {
    let last = x.icon_events.last().map(|e| e.0);
    let working = new.transcribing || StatusLine::new(&new.status, "").finishing;
    // The daemon says the dictation came to nothing. An older daemon writes
    // no outcome; then a stop with nothing to finish counts as a cancel.
    let nothing = match new.outcome {
        Some(o) => o != Outcome::Done,
        None => !working && new.status.is_empty() && x.finish_at.is_none(),
    };
    let mut push = |e: IconEvent| x.icon_events.push((e, t));
    match last {
        None if new.recording => push(IconEvent::Record),
        Some(IconEvent::Record) if !new.recording && (working || old.recording) => {
            push(IconEvent::Stop);
            if working {
                push(IconEvent::Finish);
            } else if nothing {
                push(IconEvent::Cancel);
            }
        }
        Some(IconEvent::Stop) if working => push(IconEvent::Finish),
        Some(IconEvent::Stop | IconEvent::Finish) if x.final_at.is_none() && nothing => {
            push(IconEvent::Cancel)
        }
        Some(IconEvent::Finish) if x.final_at.is_some() => push(IconEvent::Done),
        _ => {}
    }
}
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
    /// The header's height: the larger rolling digits over their captions,
    /// the title or the status icon, whichever is tallest.
    fn type_header_height(look: &Look) -> f32 {
        let stat = look.mono_line(look.value_px * STAT_SCALE)
            + 2.0
            + look.mono_line(look.caption_small_px);
        stat.max(look.mono_line(look.caption_px)).max(ICON_BOX)
    }

    /// The panel's height with `lines` text lines: the swiss panel with this
    /// look's taller header.
    pub(crate) fn type_panel_height(look: &Look, lines: u32) -> f32 {
        look.panel_height(lines) - look.header_height() + Self::type_header_height(look)
    }

    /// The surface: room for the panel at its tallest.
    pub(crate) fn type_surface(look: &Look) -> Size {
        Size::new(
            look.width.ceil(),
            Self::type_panel_height(look, swiss::MAX_LINES as u32).ceil() + 2.0,
        )
    }

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
        let h = Self::type_panel_height(l, self.lines);
        // A cancelled dictation erases the frame; a finished one fades.
        let cancelled = self.came_to_nothing();
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
            .child(self.type_cast(h))
            .children(self.limit_line(
                Point::new(
                    PAD_X + BORDER,
                    PAD_TOP
                        + BORDER
                        + Self::type_header_height(l)
                        + 2.0 * WAVE_MARGIN
                        + swiss::WAVE_HEIGHT_PX as f32,
                ),
                Size::new(l.width, h),
                l.foreground.fade(0.5),
                Color::hex(DOUBT_COLOR, 1.0),
            ))
            .child(self.type_header())
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
        if let Some(broken) = self.paths_broken_frame(&border.contours[0], l.width, h, BORDER) {
            return broken;
        }
        let exit = if cancelled {
            // Erased from where it started.
            Transition::NONE.trim(1.0, 1.0)
        } else {
            Transition::fade()
        };
        at(
            Element::stroke_path(
                "frame",
                border,
                view,
                StrokeStyle::solid(self.exhale_width(BORDER), l.border),
            ),
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

    fn type_header(&self) -> Element<Msg> {
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
        let clock = digit_roll(
            "clock",
            &self.clock,
            value,
            self.exhale_color(l.foreground, l.inactive),
            roll,
        );
        let words = digit_roll("count", &self.tally_count(), value, l.foreground, roll);
        // The pace leans forward as you speak faster and stands up when you
        // pause (MO-66).
        let (wpm, lean) = self.tally_pace();
        let cell = self.measure.layout("0", &value, None).size.w;
        let pace = tally::lean_digits("pace", &wpm.to_string(), value, l.foreground, cell, lean);
        let rule = |key: &'static str| {
            Element::shape(key, ShapeKind::RRect)
                .width(1.0)
                .fill(l.border)
                .margin_edges(0.0, 16.0, 0.0, 16.0)
                .layout(|s| s.align_self = Some(AlignSelf::STRETCH))
        };
        let cast = self.motion("count-cast");
        let mut row = Element::row("header")
            .align_items(AlignItems::START)
            .child(self.type_icon())
            .child(
                caption("title", "VOICE DICTATION", l.caption_px, 2.0)
                    .margin_edges(0.0, 0.0, 0.0, 8.0),
            )
            .child(Element::row("spacer").grow(1.0));
        if self.motion("lean-pace") {
            row = row
                .child(stat("pace", pace, "WPM"))
                .child(rule("pace-rule"));
        }
        row.child(stat("time", clock, "DURATION"))
            .child(rule("rule"))
            .child(stat("words", words, "WORDS").margin_edges(
                0.0,
                0.0,
                0.0,
                if cast { NOTCH_ROOM } else { 0.0 },
            ))
    }

    /// The word's cast and the notch by the word count (MO-65), under the
    /// rest of the panel.
    fn type_cast(&self, h: f32) -> Element<Msg> {
        let l = &self.look;
        if !self.motion("count-cast") {
            return at(Element::layer("cast"), 0.0, 0.0);
        }
        let value = l.mono(l.value_px * STAT_SCALE);
        let digits = self.tally_count().len().max(1) as f32;
        let digits_w = self.measure.layout("0", &value, None).size.w * digits;
        // The digits are right-aligned in their column, the notch just
        // left of them.
        let right = l.width - PAD_X - BORDER;
        let line = l.mono_line(l.value_px * STAT_SCALE);
        let notch = Rect::new(
            right - digits_w - NOTCH_ROOM + 2.0,
            PAD_TOP + BORDER + 2.0,
            2.5,
            line - 4.0,
        );
        let origin = Point::new(
            PAD_X + BORDER,
            PAD_TOP
                + BORDER
                + Self::type_header_height(l)
                + 2.0 * WAVE_MARGIN
                + swiss::WAVE_HEIGHT_PX as f32,
        );
        let casts = self.tally_casts(origin, notch, l.accent, false);
        at(
            Element::canvas("cast", casts.until(), move |t, r, p| casts.draw(t, r, p)),
            0.0,
            0.0,
        )
        .size(l.width, h)
    }

    /// The dictation ended without text: cancelled, no speech heard, or
    /// failed. A stop with no final pass still counts as an ending with text.
    pub(crate) fn came_to_nothing(&self) -> bool {
        matches!(
            self.ending(),
            Some(Ending::Cancelled | Ending::Empty | Ending::Failed)
        )
    }

    /// The framework's status icon, replayed from the events so far.
    fn type_icon(&self) -> Element<Msg> {
        let l = &self.look;
        let mut icon = StatusIcon::new(IconStyle {
            record: l.accent,
            work: l.foreground,
            done: l.accent,
            cancel: l.inactive,
            width: 0.08,
            done_secs: Some(DONE_SECS),
        });
        for &(e, t) in &self.extra.icon_events {
            match e {
                IconEvent::Record => icon.record(t),
                IconEvent::Stop => icon.stop(t),
                IconEvent::Finish => icon.finish(t),
                IconEvent::Done => icon.done(t),
                IconEvent::Cancel => icon.cancel(t),
            }
        }
        let until = icon.until();
        let icon = Arc::new(icon);
        Element::canvas("icon", until, move |t, r, p| {
            let centre = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            icon.paint(t, centre, r.w.min(r.h), p);
        })
        .size(ICON_BOX, ICON_BOX)
    }

    /// Notes which words the engine is unsure of and when it became sure,
    /// after the word facts changed.
    pub(crate) fn note_doubtful(&mut self, t: f64) {
        for w in self.model.words() {
            let conf = self.model.info(w.id).and_then(|i| i.conf);
            match conf {
                Some(c) if c < DOUBT => {
                    self.extra.doubtful.insert(w.id, None);
                }
                Some(_) => {
                    if let Some(sure) = self.extra.doubtful.get_mut(&w.id) {
                        sure.get_or_insert(t);
                    }
                }
                None => {}
            }
        }
    }

    /// Wavy underlines under the words on screen the engine was unsure of,
    /// placed from the same layout the live text uses.
    fn doubt_lines(&self) -> Vec<Element<Msg>> {
        let l = &self.look;
        // graded-doubt draws its own, graded line in place of this one.
        if self.extra.doubtful.is_empty() || l.on(model_signals::GRADED_DOUBT) {
            return Vec::new();
        }
        let style = l.text_style();
        let joined = self
            .shown
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let laid = self.measure.layout(&joined, &style, Some(l.text_width()));
        let mut lines = Vec::new();
        let mut start = 0usize;
        for w in &self.shown {
            let range = start..start + w.text.len();
            start = range.end + 1;
            let Some(sure) = self.extra.doubtful.get(&w.id) else {
                continue;
            };
            let Some(first) = laid
                .glyphs
                .iter()
                .find(|g| range.contains(&(g.cluster as usize)))
            else {
                continue;
            };
            let width = self.measure.layout(&w.text, &style, None).size.w;
            // Sure now, or the final pass has begun: the wave straightens,
            // then the line fades.
            let settled = sure.is_some() || self.extra.finish_at.is_some();
            let line = Element::wavy_line(
                format!("doubt-{}", w.id),
                StrokeStyle::solid(WAVE_WIDTH, Color::hex(DOUBT_COLOR, 1.0)),
                if settled { 0.0 } else { WAVE_AMP },
                WAVE_LEN,
            )
            .size(width, 2.0 * WAVE_AMP + WAVE_WIDTH + 2.0)
            .opacity(if settled { 0.0 } else { 0.9 })
            .motion(Prop::Trim, Spring::critical_to_98(0.25))
            .motion(Prop::Wave, Spring::critical_to_98(0.2))
            .motion(Prop::Opacity, Spring::critical_to_98(0.25))
            // The text reflows at once, so the line jumps with its word.
            .motion(Prop::Position, Motion::Snap)
            .motion(Prop::Size, Motion::Snap)
            .presence(Transition::NONE.trim(0.0, 0.0), Transition::fade());
            lines.push(at(line, first.pos.x, first.pos.y + WAVE_DROP - WAVE_AMP));
        }
        lines
    }

    fn type_text(&self) -> Element<Msg> {
        let l = &self.look;
        // Each new word rises into place, readable within 120 ms; a
        // revised word dips and settles.
        // A tentative word already sits at 75 % ink (confidence-ink), so it
        // enters from nearly full opacity: the reading line asks 70 % of the
        // settled ink from the first frame.
        let enter_ink = if self.look.on(crate::channels::CONFIDENCE) { 0.95 } else { 0.0 };
        let words = Element::live_text("words", &self.shown[..self.room()])
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .word_revise(revision::revise(&self.look))
            .text_style(l.text_style())
            .color(l.foreground)
            .motion(Prop::Opacity, Spring::critical_to_98(0.12))
            .motion(Prop::Offset, Spring::POP)
            .word_enter(self.paths_word_enter(Transition::fade().opacity(enter_ink).offset(0.0, WORD_RISE)));
        let words = self.typo_words(words);
        // Before the hand-off, which may wrap the live text.
        let words = self.letter_fx_words(words);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            l.foreground,
            l.accent,
        );
        let mut text = Element::column("text")
            .height(self.lines as f32 * l.text_line)
            .child(words);
        for overlay in self.paths_overlays() {
            text = text.child(overlay);
        }
        for line in self.doubt_lines() {
            text = text.child(line);
        }
        text = text.children(self.line_marks(
            l.text_style(),
            l.text_width(),
            endings::Ink::of(l),
            endings::Cancel::Unthread,
        ));
        let text = self.typo_marks(text, l.text_width());
        let text = self.signal_marks(
            text,
            &self.shown[..self.room()],
            l.text_width(),
            (0.0, 0.0),
            l.foreground,
        );
        let text = self.with_hearing(
            text,
            &l.text_style(),
            l.text_width(),
            l.foreground,
            l.accent,
        );
        let text = self.letter_fx_layers(text, l.text_width());
        self.text_area(text)
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
