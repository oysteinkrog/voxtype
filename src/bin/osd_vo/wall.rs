//! The theme wall, a development mode of the vo popup: many looks side by
//! side on one surface, every one fed the same dictation. By default a
//! scripted dictation loops (a full one with its final text, then a
//! cancelled one); with `--wall-script live` the daemon's socket and runtime
//! files drive it, so one can dictate and watch every look at once.
//!
//! Each cell runs a whole `Popup` with its own `Look`, so a look needs
//! nothing to take part: it draws its `Popup::view` at its native size
//! (`Look::surface`), and the wall scales that into the cell, clips it there
//! and labels it. Every cell gets every message; a cell that closes stays
//! empty until the next dictation starts.

use super::*;

use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length, JustifyContent};

/// Space round the grid and between cells, in logical pixels.
const PAD: f32 = 24.0;
const GAP: f32 = 16.0;
/// Room under each cell for its label.
const LABEL_H: f32 = 22.0;
const LABEL_PX: f32 = 11.0;
/// Largest scale a look is drawn at: small looks grow to fill their cell,
/// up to this.
const MAX_SCALE: f32 = 1.5;
/// Pause between one scripted dictation and the next, in seconds.
const SCRIPT_GAP: f64 = 0.8;

/// The mock-up a look comes from, for the label under its cell.
pub(crate) fn source(theme: OsdTheme) -> &'static str {
    match theme {
        OsdTheme::Default | OsdTheme::Swiss => "concept 12",
        OsdTheme::Showcase => "families 03 09 12 17 21 22",
        OsdTheme::Type => "concept 12, family 12",
        OsdTheme::Jarvis => "concept 02",
        OsdTheme::Cyberpunk => "concept 11",
        OsdTheme::Synthwave => "concept 04",
        OsdTheme::Mecha => "concept 23",
        OsdTheme::Island => "concept 03, family 03",
        OsdTheme::Karaoke => "concept 14",
        OsdTheme::Captions => "concept 13",
        OsdTheme::Scope => "concept 18",
        // A look added later shows no source until it gets a row here.
        #[allow(unreachable_patterns)]
        _ => "",
    }
}

/// What drives the wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    /// The scripted dictations, looping.
    Demo,
    /// The daemon's socket and runtime files.
    Live,
}

pub(crate) struct Wall {
    looks: Vec<Look>,
    fonts: FontRegistry,
    cells: Vec<Popup>,
    cols: usize,
    size: Size,
    script: Script,
    /// One loop of the demo: the messages and when they come, from the
    /// start of the loop, and the loop's length.
    demo: Vec<(f64, Msg)>,
    demo_len: f64,
    /// When the current loop of the demo ends.
    loop_end: f64,
}

impl Wall {
    pub(crate) fn new(
        looks: Vec<Look>,
        fonts: &FontRegistry,
        cols: usize,
        size: Size,
        script: Script,
    ) -> Wall {
        let (demo, demo_len) = demo_script(&looks[0]);
        let mut wall = Wall {
            looks,
            fonts: fonts.clone(),
            cells: Vec::new(),
            cols: cols.max(1),
            size,
            script,
            demo,
            demo_len,
            loop_end: 0.0,
        };
        wall.cells = wall.fresh_cells();
        wall
    }

    /// One loop of the demo, in seconds.
    pub(crate) fn demo_len(&self) -> f64 {
        self.demo_len
    }

    fn fresh_cells(&self) -> Vec<Popup> {
        self.looks
            .iter()
            .map(|l| {
                let mut p = Popup::new(l.clone(), &self.fonts);
                p.in_wall = true;
                p
            })
            .collect()
    }

    /// New popups in every cell, as for a new dictation.
    fn restart(&mut self, cx: &mut Cx<Msg>) {
        self.cells = self.fresh_cells();
        for p in &mut self.cells {
            p.init(cx);
        }
    }

    /// One loop of the demo from now, ending with `Msg::Loop`.
    fn schedule_demo(&mut self, cx: &mut Cx<Msg>) {
        for (t, m) in &self.demo {
            cx.after(*t, m.clone());
        }
        cx.after(self.demo_len, Msg::Loop);
        self.loop_end = cx.t() + self.demo_len;
    }
}

/// The full dictation, then the cancelled one, which starts with
/// `Msg::Loop` so the cells start afresh.
fn demo_script(look: &Look) -> (Vec<(f64, Msg)>, f64) {
    let mut out = Vec::new();
    let mut at = 0.0;
    for cancel in [false, true] {
        let s = scenario(look, cancel);
        if cancel {
            out.push((at, Msg::Loop));
        }
        for (t, input) in s.inputs {
            if let Input::Msg(m) = input {
                out.push((at + t, m));
            }
        }
        at += s.duration + SCRIPT_GAP;
    }
    (out, at)
}

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

impl App for Wall {
    type Msg = Msg;

    fn init(&mut self, cx: &mut Cx<Msg>) {
        for p in &mut self.cells {
            p.init(cx);
        }
        if self.script == Script::Demo {
            self.schedule_demo(cx);
        }
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        match msg {
            Msg::Loop => {
                self.restart(cx);
                if self.script == Script::Demo && cx.t() >= self.loop_end - 1e-3 {
                    self.schedule_demo(cx);
                }
            }
            Msg::Files(files) => {
                // Live: a new recording after the cells closed starts them
                // afresh.
                if self.script == Script::Live
                    && files.recording
                    && self.cells.iter().all(|p| p.closing)
                {
                    self.restart(cx);
                }
                for p in &mut self.cells {
                    p.update(Msg::Files(files.clone()), cx);
                }
            }
            Msg::Tick => {
                // A cell re-arms its ambient tick whenever any Tick arrives,
                // so cancel the ones still pending first, or they pile up
                // with every tick of every cell.
                for p in &mut self.cells {
                    if let Some(id) = p.extra.tick.take() {
                        cx.cancel(id);
                    }
                }
                for p in &mut self.cells {
                    p.update(Msg::Tick, cx);
                }
            }
            m => {
                for p in &mut self.cells {
                    p.update(m.clone(), cx);
                }
            }
        }
    }

    fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
        let n = self.cells.len();
        let cols = self.cols.min(n.max(1));
        let rows = n.div_ceil(cols).max(1);
        let cell_w = (self.size.w - 2.0 * PAD - (cols - 1) as f32 * GAP) / cols as f32;
        let cell_h = (self.size.h - 2.0 * PAD - (rows - 1) as f32 * GAP) / rows as f32 - LABEL_H;
        let label_style = self.looks[0].mono(LABEL_PX).tracking(1.0);
        let mut wall = Element::panel("wall")
            .size(self.size.w, self.size.h)
            .radius(12.0)
            .fill(Color::rgba(0.04, 0.05, 0.07, 0.9));
        for (i, (p, look)) in self.cells.iter().zip(&self.looks).enumerate() {
            let (c, r) = (i % cols, i / cols);
            let native = look.surface();
            let s = (cell_w / native.w).min(cell_h / native.h).min(MAX_SCALE);
            // The look at its native size, scaled about its centre into the
            // middle of the cell.
            let theme = at(
                Element::layer("theme")
                    .size(native.w, native.h)
                    .scale(s)
                    .motion(Prop::Scale, Motion::Snap)
                    .child(p.view(tokens)),
                (cell_w - native.w) / 2.0,
                (cell_h - native.h) / 2.0,
            );
            let name = format!("{:?}", look.theme).to_lowercase();
            let src = source(look.theme);
            let label = if src.is_empty() {
                name
            } else {
                format!("{name}  ·  {src}")
            };
            let cell = Element::column(format!("cell-{i}"))
                .size(cell_w, cell_h + LABEL_H)
                .child(
                    Element::layer("clip")
                        .clip()
                        .size(cell_w, cell_h)
                        .child(theme),
                )
                .child(
                    Element::label("label", label)
                        .text_style(label_style)
                        .color(look.inactive)
                        .margin_edges(6.0, 0.0, 0.0, 0.0),
                );
            wall = wall.child(at(
                cell,
                PAD + c as f32 * (cell_w + GAP),
                PAD + r as f32 * (cell_h + LABEL_H + GAP),
            ));
        }
        // Centred on the output: the wall covers it, but lays out to its
        // own size.
        Element::column("screen")
            .align_items(AlignItems::CENTER)
            .justify(JustifyContent::CENTER)
            .child(wall)
    }
}
