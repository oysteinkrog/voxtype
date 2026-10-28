//! The `Default` and `Swiss` look of the vo popup: the panel with its header,
//! waveform, transcript, rule and status row. The other looks fall through to it
//! as well, and several reuse its pieces.

use super::*;

impl Popup {
    pub(super) fn panel(&self) -> Element<Msg> {
        let l = &self.look;
        let status = self.status();
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
            .border(l.border, self.exhale_width(BORDER))
            .motion(Prop::StrokeWidth, self.exhale_snap())
            .child(self.header())
            .child(self.waveform(&status))
            .child(self.transcript())
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .height(1.0)
                    .fill(l.border)
                    .margin_edges(RULE_MARGIN_TOP, 0.0, 0.0, 0.0),
            )
            .child(self.status_row(&status))
    }

    pub(super) fn header(&self) -> Element<Msg> {
        let l = &self.look;
        let caption = |key: &'static str, text: &str, px: f32, tracking: f32| {
            Element::label(key, text)
                .text_style(l.mono(px).tracking(tracking))
                .color(l.inactive)
        };
        let stat = |key: &'static str, value: &str, name: &'static str| {
            Element::column(key)
                .gap(2.0)
                .child(
                    self.exhale_motion(
                        Element::label("value", value)
                            .text_style(l.mono(l.value_px))
                            .color(if key == "time" {
                                self.exhale_color(l.foreground, l.inactive)
                            } else {
                                l.foreground
                            }),
                        &[Prop::TextColor],
                    ),
                )
                .child(caption("caption", name, l.caption_small_px, 1.5))
        };
        // The icon theme's microphone, or one from shapes: the capsule and its stand.
        let mic = match l.mic {
            Some((id, view)) => Element::path("mic", id, view)
                .size(ICON, ICON)
                .fill(l.accent),
            None => self.shape_mic(),
        };
        Element::row("header")
            .align_items(AlignItems::START)
            .child(mic)
            .child(
                caption("title", "VOICE DICTATION", l.caption_px, 2.0)
                    .margin_edges(0.0, 0.0, 0.0, 8.0),
            )
            .child(Element::row("spacer").grow(1.0))
            .child(stat("time", &self.clock, "DURATION"))
            .child(
                Element::shape("rule", ShapeKind::RRect)
                    .width(1.0)
                    .fill(l.border)
                    .margin_edges(0.0, 16.0, 0.0, 16.0)
                    .layout(|s| s.align_self = Some(AlignSelf::STRETCH)),
            )
            .child(stat("words", &self.word_count.to_string(), "WORDS"))
    }

    /// A microphone from shapes, for when the icon theme has none.
    pub(super) fn shape_mic(&self) -> Element<Msg> {
        let l = &self.look;
        Element::column("mic")
            .size(ICON, ICON)
            .align_items(AlignItems::CENTER)
            .gap(1.5)
            .child(
                Element::shape("body", ShapeKind::Capsule)
                    .size(6.0, 9.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("stand", ShapeKind::RRect)
                    .size(1.5, 2.0)
                    .fill(l.accent),
            )
            .child(
                Element::shape("base", ShapeKind::RRect)
                    .size(7.0, 1.5)
                    .fill(l.accent),
            )
    }

    pub(super) fn waveform(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        // Once the recording stops the frozen waveform turns grey.
        let (left, center, right) = if status.finishing {
            (l.inactive, l.inactive, l.inactive)
        } else {
            l.hues
        };
        let to_vo = |c: Color| OsdColor::rgba(c.r, c.g, c.b, c.a);
        let stops = wave_stops(to_vo(left), to_vo(center), to_vo(right))
            .into_iter()
            .map(|(p, c, a)| (p, Color::rgba(c.r, c.g, c.b, a)))
            .collect();
        let wave = Wave {
            levels: self.exhale_levels(&self.wave),
            newest_at: self.newest_at,
            rate: FRAME_HZ as f32,
            window: l.window as f32,
            stops,
            min_half: 0.5,
            baseline: Some(l.border),
        };
        Element::wave("wave", wave)
            .height(swiss::WAVE_HEIGHT_PX as f32)
            .margin_edges(WAVE_MARGIN, 0.0, WAVE_MARGIN, 0.0)
    }

    pub(super) fn transcript(&self) -> Element<Msg> {
        let l = &self.look;
        let mut words = Element::live_text("words", &self.shown)
            .line_motion(&self.look)
            .revision_motion(revision::motion(&self.look))
            .text_style(l.text_style())
            .color(l.foreground);
        // Without word_fade nothing moves: a new word is on screen at full
        // opacity in the first frame after it arrives, as in GTK.
        if l.word_fade {
            words = words
                .motion(Prop::Opacity, Spring::SNAP)
                .motion(Prop::Offset, Spring::SNAP)
                .word_enter(Transition::fade().offset(0.0, 3.0));
        }
        let words = self.typo_words(words);
        let words = self.handoff_or(
            words,
            &l.text_style(),
            Some(l.text_width()),
            l.foreground,
            l.accent,
        );
        let text = Element::column("text")
            .height(self.lines as f32 * l.text_line)
            .child(words)
            .children(self.line_marks(
                l.text_style(),
                l.text_width(),
                endings::Ink::of(l),
                endings::Cancel::Unthread,
            ));
        let text = self.typo_marks(text, l.text_width());
        let text = self.with_hearing(
            text,
            &l.text_style(),
            l.text_width(),
            l.foreground,
            l.accent,
        );
        self.text_area(text)
    }

    /// The text area. For the looks whose [`Look::margin_ellipsis`] is on,
    /// it sits in a layer clipped to its own box, widened on the left for the
    /// "…" that hangs there once earlier lines are cut, so a line that leaves
    /// while the text scrolls slides behind the edge instead of crossing the
    /// wave or the status row.
    pub(super) fn text_area(&self, text: Element<Msg>) -> Element<Msg> {
        use vo::scene::taffy::{LengthPercentage, LengthPercentageAuto};
        let l = &self.look;
        if !l.margin_ellipsis() {
            return text.margin_edges(0.0, 0.0, TEXT_MARGIN_BOTTOM, 0.0);
        }
        let style = l.text_style();
        let w = self.measure.layout("…", &style, None).size.w.ceil();
        let mut text = text;
        if self.cut > 0 && !self.shown.is_empty() && self.delivered().is_none() {
            text = text.child(
                Element::label("ellipsis", "…")
                    .text_style(style)
                    .color(l.inactive)
                    .layout(|s| {
                        s.position = vo::scene::taffy::Position::Absolute;
                        s.inset = vo::scene::taffy::Rect {
                            left: LengthPercentageAuto::length(-w),
                            top: LengthPercentageAuto::length(0.0),
                            right: LengthPercentageAuto::auto(),
                            bottom: LengthPercentageAuto::auto(),
                        };
                    }),
            );
        }
        // The clip reaches into the gap above the status row, so descenders
        // and rising words on the last line are not cut; the negative margin
        // keeps the layout as it was.
        let below = TEXT_MARGIN_BOTTOM + STATUS_MARGIN_TOP - 4.0;
        Element::layer("textclip")
            .clip()
            .margin_edges(0.0, 0.0, TEXT_MARGIN_BOTTOM - below, -w)
            .layout(|s| {
                s.padding.left = LengthPercentage::length(w);
                s.padding.bottom = LengthPercentage::length(below);
            })
            .child(text)
    }

    pub(super) fn status_row(&self, status: &StatusLine) -> Element<Msg> {
        let l = &self.look;
        // A filled square while listening, a hollow one while finishing.
        let dot = if status.finishing {
            Element::shape("dot", ShapeKind::RRect)
                .size(DOT, DOT)
                .fill(Color::TRANSPARENT)
                .border(l.accent, 1.0)
        } else {
            Element::shape("dot", ShapeKind::RRect)
                .size(DOT, DOT)
                .fill(l.accent)
        };
        let mut row = Element::row("status")
            .align_items(AlignItems::CENTER)
            .margin_edges(STATUS_MARGIN_TOP, 0.0, 0.0, 0.0)
            .child(dot)
            .child(
                Element::label("state", status.state.as_str())
                    .text_style(l.mono(l.status_px))
                    .color(l.accent)
                    .margin_edges(0.0, 0.0, 0.0, 10.0),
            )
            .child(
                Element::label("detail", status.detail.as_str())
                    .text_style(l.mono(l.status_px).clamp_lines(1))
                    .color(l.inactive)
                    .shrink(1.0)
                    .min_width(0.0),
            );
        if let Some(ruler) = self.audio_ruler(l.accent, l.border) {
            row = row.child(ruler.margin_edges(0.0, 0.0, 0.0, 16.0));
        } else if self.shows_progress(status) {
            let progress = Progress {
                fraction: self.files.fraction.map(|f| f.clamp(0.0, 1.0) as f32),
                track: l.border,
                bar: l.accent,
                since: self.shown_at,
                period: 1.6,
            };
            row = row.child(
                Element::progress("progress", progress)
                    .height(2.0)
                    .grow(1.0)
                    .margin_edges(0.0, 0.0, 0.0, 16.0),
            );
        }
        row
    }
}
