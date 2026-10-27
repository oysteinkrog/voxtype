//! The motion station (MO-90): `voxtype-osd-vo --station` shows every motion
//! exhibit on one surface, page by page. An exhibit is a popup tile labelled
//! with its MO id and name: a whole `Popup` with its own `Look`, a theme and
//! the features it switches on.
//!
//! Live (the daemon runs), every tile on the page gets the real dictation;
//! between dictations each tile loops its own moment (`moments.rs`), so an
//! exhibit about an ending replays that ending after each dictation. With
//! the demo script the tiles only loop their moments.
//!
//! Exhibits are files named `mo_NN_<slug>.rs` in this folder, each with a
//! `pub(super) fn exhibit() -> Exhibit`. `build.rs` finds and registers them,
//! so adding one touches no shared file. The contract for lanes is in the
//! station notes (`STATION.md` in the motion plan folder).

use super::*;

pub(crate) use super::moments::Moment;

use vo::scene::taffy;
use vo::scene::taffy::prelude::{auto, length, JustifyContent};

// One `mod mo_NN_slug;` per exhibit file and `EXHIBITS`, the list of their
// `exhibit` functions in file-name order.
include!(concat!(env!("OUT_DIR"), "/station_exhibits.rs"));

/// A tile's own view in place of `Popup::view`.
pub(crate) type ViewFn = fn(&Popup, &vo::Tokens) -> Element<Msg>;

/// One exhibit: what a tile shows and how.
#[derive(Clone)]
pub(crate) struct Exhibit {
    /// `MO-NN`.
    pub(crate) id: &'static str,
    /// The concept's name, as in the motion plan.
    pub(crate) name: &'static str,
    pub(crate) theme: OsdTheme,
    /// Switched on in the tile's `Look` (`Look::on`).
    pub(crate) features: Vec<&'static str>,
    /// What the tile plays while nobody dictates, in turn.
    pub(crate) moments: Vec<Moment>,
    /// Settings the look already reads.
    pub(crate) config: Option<fn(&mut OsdConfig)>,
    pub(crate) view: Option<ViewFn>,
}

// The builder is the exhibits' API: an unused method is one no exhibit
// needs yet.
#[allow(dead_code)]
impl Exhibit {
    pub(crate) fn new(id: &'static str, name: &'static str, theme: OsdTheme) -> Exhibit {
        Exhibit {
            id,
            name,
            theme,
            features: Vec::new(),
            moments: Vec::new(),
            config: None,
            view: None,
        }
    }

    /// Switches a feature on for this tile; `Look::on(name)` says so.
    pub(crate) fn feature(mut self, name: &'static str) -> Exhibit {
        self.features.push(name);
        self
    }

    /// The moment the tile loops while nobody dictates.
    pub(crate) fn moment(mut self, m: Moment) -> Exhibit {
        self.moments = vec![m];
        self
    }

    /// Several moments, played in turn.
    pub(crate) fn moments(mut self, ms: &[Moment]) -> Exhibit {
        self.moments = ms.to_vec();
        self
    }

    pub(crate) fn config(mut self, f: fn(&mut OsdConfig)) -> Exhibit {
        self.config = Some(f);
        self
    }

    pub(crate) fn view(mut self, f: ViewFn) -> Exhibit {
        self.view = Some(f);
        self
    }

    fn moment_at(&self, n: usize) -> Moment {
        if self.moments.is_empty() {
            Moment::Dictation
        } else {
            self.moments[n % self.moments.len()]
        }
    }

    /// The MO number, for sorting and filters.
    fn number(&self) -> u32 {
        self.id
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .unwrap_or(u32::MAX)
    }
}

/// Every registered exhibit, by MO number.
pub(crate) fn exhibits() -> Vec<Exhibit> {
    let mut all: Vec<Exhibit> = EXHIBITS.iter().map(|f| f()).collect();
    all.sort_by_key(|e| e.number());
    all
}

/// Keeps the exhibits `only` names: `MO-20`, `mo-20`, `20` or an exhibit's
/// name. Empty keeps all.
pub(crate) fn filter(all: Vec<Exhibit>, only: &[String]) -> Vec<Exhibit> {
    if only.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|e| {
            only.iter().any(|o| {
                let o = o.trim();
                o.eq_ignore_ascii_case(e.id)
                    || o.parse::<u32>().is_ok_and(|n| n == e.number())
                    || o.eq_ignore_ascii_case(e.name)
            })
        })
        .collect()
}

/// What drives the station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    Demo,
    Live,
}

/// The station's own timers. They come back as `Msg::Station`.
#[derive(Clone, Debug)]
pub(crate) enum Cue {
    /// Scripted messages are due.
    Pump,
    /// Time to turn the page, if this is still the page's generation.
    Page(u64),
    /// A live dictation may have ended; go back to the moments.
    Resume(u64),
}

/// Space round the grid and between tiles, the index bar and the labels, in
/// logical pixels.
const PAD: f32 = 28.0;
const GAP: f32 = 18.0;
const BAR_H: f32 = 56.0;
const LABEL_H: f32 = 30.0;
const ID_PX: f32 = 15.0;
const NAME_PX: f32 = 14.0;
const SMALL_PX: f32 = 12.0;
const MAX_SCALE: f32 = 1.25;
/// Between one loop of a tile's moment and the next.
const LOOP_GAP: f64 = 0.8;
/// After a live dictation ends, before the tiles go back to their moments.
const RESUME_AFTER: f64 = 2.0;

/// One tile on the page.
struct Tile {
    exhibit: Exhibit,
    look: Look,
    popup: Popup,
    /// Scripted messages still to come, at scene times, in order.
    queue: VecDeque<(f64, Msg)>,
    /// When this loop of the moment ends, and how many loops have played.
    loop_end: f64,
    loops: usize,
}

pub(crate) struct Station {
    exhibits: Vec<Exhibit>,
    cfg: OsdConfig,
    theme: PlasmaTheme,
    stop_hint: String,
    fonts: FontRegistry,
    /// Features every tile gets (`--motion`, `--station-reduced`).
    extra: Vec<String>,
    /// For the index bar and labels.
    chrome: Look,
    script: Script,
    size: Size,
    cols: usize,
    rows: usize,
    dwell: f64,
    /// A page held by `--station-page`.
    hold: Option<usize>,
    page: usize,
    tiles: Vec<Tile>,
    /// Changes with each page, so a stale page timer does nothing.
    page_gen: u64,
    page_at: f64,
    /// A live dictation has the tiles; changes with each, for `Cue::Resume`.
    live: bool,
    live_gen: u64,
    /// A `Cue::Resume` is pending for this live dictation.
    resume_armed: bool,
    pump: Option<vo::TimerId>,
}

pub(crate) struct Setup {
    pub(crate) exhibits: Vec<Exhibit>,
    pub(crate) cfg: OsdConfig,
    pub(crate) theme: PlasmaTheme,
    pub(crate) stop_hint: String,
    pub(crate) fonts: FontRegistry,
    pub(crate) extra: Vec<String>,
    pub(crate) script: Script,
    pub(crate) size: Size,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) dwell: f64,
    pub(crate) page: Option<usize>,
}

impl Station {
    pub(crate) fn new(s: Setup) -> Station {
        let chrome = Look::new(&s.cfg, &s.theme, String::new(), &s.fonts);
        let per = (s.cols * s.rows).max(1);
        let pages = s.exhibits.len().div_ceil(per).max(1);
        let hold = s.page.map(|p| p.clamp(1, pages) - 1);
        Station {
            exhibits: s.exhibits,
            cfg: s.cfg,
            theme: s.theme,
            stop_hint: s.stop_hint,
            fonts: s.fonts,
            extra: s.extra,
            chrome,
            script: s.script,
            size: s.size,
            cols: s.cols.max(1),
            rows: s.rows.max(1),
            dwell: s.dwell.max(3.0),
            hold,
            page: hold.unwrap_or(0),
            tiles: Vec::new(),
            page_gen: 0,
            page_at: 0.0,
            live: false,
            live_gen: 0,
            resume_armed: false,
            pump: None,
        }
    }

    fn per_page(&self) -> usize {
        self.cols * self.rows
    }

    pub(crate) fn pages(&self) -> usize {
        self.exhibits.len().div_ceil(self.per_page()).max(1)
    }

    /// How long one page shows, for an offline render.
    pub(crate) fn dwell(&self) -> f64 {
        self.dwell
    }

    fn look_for(&self, e: &Exhibit) -> Look {
        let mut c = self.cfg.clone();
        c.theme = e.theme;
        if let Some(f) = e.config {
            f(&mut c);
        }
        let mut look = Look::new(&c, &self.theme, self.stop_hint.clone(), &self.fonts);
        let mut features: Vec<String> = e.features.iter().map(|f| f.to_string()).collect();
        features.extend(self.extra.iter().cloned());
        look.features = features.into();
        look
    }

    fn fresh_popup(&self, look: &Look) -> Popup {
        let mut p = Popup::new(look.clone(), &self.fonts);
        p.in_wall = true;
        p
    }

    /// The tiles for the current page, each starting its first moment.
    fn open_page(&mut self, cx: &mut Cx<Msg>) {
        self.page_gen += 1;
        self.page_at = cx.t();
        let start = self.page * self.per_page();
        let exhibits: Vec<Exhibit> = self
            .exhibits
            .iter()
            .skip(start)
            .take(self.per_page())
            .cloned()
            .collect();
        self.tiles = exhibits
            .into_iter()
            .map(|e| {
                let look = self.look_for(&e);
                Tile {
                    popup: self.fresh_popup(&look),
                    exhibit: e,
                    look,
                    queue: VecDeque::new(),
                    loop_end: 0.0,
                    loops: 0,
                }
            })
            .collect();
        for i in 0..self.tiles.len() {
            if self.live {
                self.tiles[i].popup.init(cx);
            } else {
                self.start_moment(i, cx);
            }
        }
        if self.hold.is_none() && self.pages() > 1 {
            cx.after(self.dwell, Msg::Station(Box::new(Cue::Page(self.page_gen))));
        }
        self.arm_pump(cx);
    }

    /// A fresh popup in tile `i`, playing its next moment from now.
    fn start_moment(&mut self, i: usize, cx: &mut Cx<Msg>) {
        let now = cx.t();
        let popup = self.fresh_popup(&self.tiles[i].look);
        let tile = &mut self.tiles[i];
        tile.popup = popup;
        tile.popup.init(cx);
        let m = tile.exhibit.moment_at(tile.loops);
        tile.loops += 1;
        let s = super::moments::scenario(&tile.look, m);
        tile.queue = s
            .inputs
            .into_iter()
            .filter_map(|(t, input)| match input {
                Input::Msg(m) => Some((now + t, m)),
                _ => None,
            })
            .collect();
        tile.loop_end = now + s.duration + LOOP_GAP;
    }

    /// Wakes the station when the next scripted message is due.
    fn arm_pump(&mut self, cx: &mut Cx<Msg>) {
        if let Some(id) = self.pump.take() {
            cx.cancel(id);
        }
        if self.live {
            return;
        }
        let next = self
            .tiles
            .iter()
            .flat_map(|t| t.queue.front().map(|q| q.0).into_iter().chain([t.loop_end]))
            .min_by(f64::total_cmp);
        if let Some(at) = next {
            let wait = (at - cx.t()).max(0.0);
            self.pump = Some(cx.after(wait, Msg::Station(Box::new(Cue::Pump))));
        }
    }

    /// Hands each tile its due scripted messages and restarts the tiles
    /// whose loop ended.
    fn pump(&mut self, cx: &mut Cx<Msg>) {
        let now = cx.t() + 1e-6;
        for i in 0..self.tiles.len() {
            while self.tiles[i].queue.front().is_some_and(|q| q.0 <= now) {
                let (_, m) = self.tiles[i].queue.pop_front().unwrap();
                self.tiles[i].popup.update(m, cx);
            }
            if self.tiles[i].queue.is_empty() && self.tiles[i].loop_end <= now {
                self.start_moment(i, cx);
            }
        }
        self.arm_pump(cx);
    }

    /// A message from the daemon: does it start a dictation?
    fn starts_dictation(msg: &Msg) -> bool {
        match msg {
            Msg::Files(f) => (f.recording || f.finishing || f.transcribing) && !f.suppressed,
            Msg::Audio(frames) => frames.iter().any(|f| !f.is_pump_silence()),
            _ => false,
        }
    }

    /// The tiles leave their moments for the live dictation.
    fn go_live(&mut self, cx: &mut Cx<Msg>) {
        self.live = true;
        self.live_gen += 1;
        self.resume_armed = false;
        for i in 0..self.tiles.len() {
            let popup = self.fresh_popup(&self.tiles[i].look);
            let tile = &mut self.tiles[i];
            tile.queue.clear();
            tile.popup = popup;
            tile.popup.init(cx);
        }
        self.arm_pump(cx);
    }

    fn live_msg(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        if !self.live {
            if !Self::starts_dictation(&msg) {
                return;
            }
            self.go_live(cx);
        } else if Self::starts_dictation(&msg)
            && matches!(&msg, Msg::Files(f) if f.recording)
            && self.tiles.iter().all(|t| t.popup.closing)
        {
            // A new recording after the last one closed.
            self.go_live(cx);
        }
        for t in &mut self.tiles {
            t.popup.update(msg.clone(), cx);
        }
        if !self.resume_armed && self.tiles.iter().all(|t| t.popup.closing) {
            self.resume_armed = true;
            let gen = self.live_gen;
            cx.after(RESUME_AFTER, Msg::Station(Box::new(Cue::Resume(gen))));
        }
    }

    fn resume(&mut self, gen: u64, cx: &mut Cx<Msg>) {
        if !self.live || gen != self.live_gen {
            return;
        }
        self.resume_armed = false;
        if !self.tiles.iter().all(|t| t.popup.closing) {
            return;
        }
        self.live = false;
        for i in 0..self.tiles.len() {
            self.start_moment(i, cx);
        }
        self.arm_pump(cx);
    }

    fn turn_page(&mut self, gen: u64, cx: &mut Cx<Msg>) {
        if gen != self.page_gen {
            return;
        }
        if self.live {
            // Never mid-dictation: look again shortly.
            cx.after(2.0, Msg::Station(Box::new(Cue::Page(gen))));
            return;
        }
        self.page = (self.page + 1) % self.pages();
        self.open_page(cx);
    }
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

impl App for Station {
    type Msg = Msg;

    fn init(&mut self, cx: &mut Cx<Msg>) {
        self.open_page(cx);
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        match msg {
            Msg::Station(cue) => match *cue {
                Cue::Pump => {
                    self.pump = None;
                    self.pump(cx);
                }
                Cue::Page(gen) => self.turn_page(gen, cx),
                Cue::Resume(gen) => self.resume(gen, cx),
            },
            m @ (Msg::Audio(_) | Msg::Voice(_) | Msg::Files(_)) => {
                if self.script == Script::Live {
                    self.live_msg(m, cx);
                }
            }
            Msg::Tick => {
                // A tile re-arms its ambient tick on any Tick: cancel the
                // pending ones first, or they pile up (see wall.rs).
                for t in &mut self.tiles {
                    if let Some(id) = t.popup.extra.tick.take() {
                        cx.cancel(id);
                    }
                }
                for t in &mut self.tiles {
                    t.popup.update(Msg::Tick, cx);
                }
            }
            m => {
                for t in &mut self.tiles {
                    t.popup.update(m.clone(), cx);
                }
            }
        }
    }

    fn view(&self, tokens: &vo::Tokens) -> Element<Msg> {
        let l = &self.chrome;
        let cols = self.cols;
        let rows = self.rows;
        let grid_top = PAD + BAR_H + GAP;
        let cell_w = (self.size.w - 2.0 * PAD - (cols - 1) as f32 * GAP) / cols as f32;
        let cell_h =
            (self.size.h - grid_top - PAD - (rows - 1) as f32 * GAP) / rows as f32 - LABEL_H;
        let fg = Color::rgba(0.92, 0.94, 0.97, 1.0);
        let dim = Color::rgba(0.62, 0.66, 0.72, 1.0);
        let accent = Color::rgba(0.35, 0.78, 1.0, 1.0);
        let mut screen = Element::panel("station")
            .size(self.size.w, self.size.h)
            .fill(Color::rgba(0.035, 0.04, 0.055, 0.96));
        screen = screen.child(at(self.index_bar(tokens), PAD, PAD));
        for (i, tile) in self.tiles.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let native = tile.look.surface();
            let s = (cell_w / native.w).min(cell_h / native.h).min(MAX_SCALE);
            let body = match tile.exhibit.view {
                Some(f) => f(&tile.popup, tokens),
                None => tile.popup.view(tokens),
            };
            let theme = at(
                Element::layer("theme")
                    .size(native.w, native.h)
                    .scale(s)
                    .motion(Prop::Scale, Motion::Snap)
                    .default_motion(default_motion(&tile.look))
                    .child(body),
                (cell_w - native.w) / 2.0,
                (cell_h - native.h) / 2.0,
            );
            let theme_name = format!("{:?}", tile.look.theme).to_lowercase();
            let moment = if self.live {
                "live".to_string()
            } else {
                tile.exhibit
                    .moment_at(tile.loops.saturating_sub(1))
                    .name()
                    .to_string()
            };
            let label = Element::row("label")
                .size(cell_w, LABEL_H)
                .align_items(AlignItems::CENTER)
                .child(
                    Element::label("id", tile.exhibit.id)
                        .text_style(l.mono(ID_PX).tracking(0.5))
                        .color(accent)
                        .margin_edges(0.0, 10.0, 0.0, 0.0),
                )
                .child(
                    Element::label("name", tile.exhibit.name)
                        .text_style(TextStyle::new(
                            SANS,
                            NAME_PX,
                            l.text_line.max(NAME_PX * 1.3),
                        ))
                        .color(fg),
                )
                .child(Element::column("spacer").layout(|s| s.flex_grow = 1.0))
                .child(
                    Element::label("meta", format!("{theme_name} · {moment}"))
                        .text_style(l.mono(SMALL_PX))
                        .color(dim),
                );
            let cell = Element::column(format!("cell-{i}"))
                .size(cell_w, cell_h + LABEL_H)
                .child(label)
                .child(
                    Element::layer("clip")
                        .clip()
                        .size(cell_w, cell_h)
                        .radius(8.0)
                        .fill(Color::rgba(0.07, 0.08, 0.1, 0.9))
                        .child(theme),
                );
            screen = screen.child(at(
                cell,
                PAD + c as f32 * (cell_w + GAP),
                grid_top + r as f32 * (cell_h + LABEL_H + GAP),
            ));
        }
        Element::column("screen")
            .align_items(AlignItems::CENTER)
            .justify(JustifyContent::CENTER)
            .child(screen)
    }
}

impl Station {
    /// The bar along the top: where the station is, every page's MO ids and
    /// the time to the next page.
    fn index_bar(&self, _: &vo::Tokens) -> Element<Msg> {
        let l = &self.chrome;
        let w = self.size.w - 2.0 * PAD;
        let fg = Color::rgba(0.92, 0.94, 0.97, 1.0);
        let dim = Color::rgba(0.55, 0.6, 0.66, 1.0);
        let accent = Color::rgba(0.35, 0.78, 1.0, 1.0);
        let mode = match (self.script, self.live) {
            (Script::Live, true) => "LIVE: dictating",
            (Script::Live, false) => "LIVE: waiting, moments loop",
            (Script::Demo, _) => "DEMO script",
        };
        let mut row = Element::row("pages")
            .align_items(AlignItems::CENTER)
            .child(
                Element::label("title", "MOTION STATION")
                    .text_style(l.mono(16.0).tracking(2.0))
                    .color(fg)
                    .margin_edges(0.0, 18.0, 0.0, 0.0),
            )
            .child(
                Element::label("mode", mode)
                    .text_style(l.mono(SMALL_PX))
                    .color(if self.live { accent } else { dim })
                    .margin_edges(0.0, 24.0, 0.0, 0.0),
            );
        let per = self.per_page();
        for p in 0..self.pages() {
            let first = self.exhibits.get(p * per).map_or("", |e| e.id);
            let last = self
                .exhibits
                .get(((p + 1) * per).min(self.exhibits.len()).saturating_sub(1))
                .map_or("", |e| e.id);
            let current = p == self.page;
            let chip = Element::panel(format!("page-{p}"))
                .pad_edges(5.0, 10.0, 5.0, 10.0)
                .radius(4.0)
                .margin_edges(0.0, 8.0, 0.0, 0.0)
                .fill(if current {
                    Color::rgba(0.35, 0.78, 1.0, 0.22)
                } else {
                    Color::rgba(1.0, 1.0, 1.0, 0.05)
                })
                .child(
                    Element::label("text", format!("{}  {first} to {last}", p + 1))
                        .text_style(l.mono(SMALL_PX))
                        .color(if current { fg } else { dim }),
                );
            row = row.child(chip);
        }
        let status = match self.hold {
            Some(_) => format!("page {} of {}, held", self.page + 1, self.pages()),
            None => format!(
                "page {} of {}, {} exhibits",
                self.page + 1,
                self.pages(),
                self.exhibits.len()
            ),
        };
        row = row
            .child(Element::column("spacer").layout(|s| s.flex_grow = 1.0))
            .child(
                Element::label("status", status)
                    .text_style(l.mono(SMALL_PX))
                    .color(dim),
            );
        // How far the page has come towards the next.
        let done = if self.hold.is_some() || self.pages() < 2 {
            0.0
        } else {
            ((self.chrome_now() - self.page_at) / self.dwell).clamp(0.0, 1.0) as f32
        };
        Element::column("bar")
            .size(w, BAR_H)
            .justify(JustifyContent::CENTER)
            .child(row.size(w, BAR_H - 6.0))
            .child(
                Element::panel("track")
                    .size(w, 2.0)
                    .fill(Color::rgba(1.0, 1.0, 1.0, 0.06))
                    .child(Element::panel("done").size(w * done, 2.0).fill(accent)),
            )
    }

    /// Scene time as the popups last saw it.
    fn chrome_now(&self) -> f64 {
        self.tiles
            .iter()
            .map(|t| t.popup.extra.now)
            .fold(self.page_at, f64::max)
    }
}

/// `voxtype-osd-vo --station`.
pub(crate) fn run(
    args: &Args,
    cfg: &OsdConfig,
    theme: &PlasmaTheme,
    stop_hint: String,
    fonts: FontRegistry,
) -> anyhow::Result<()> {
    let exhibits = filter(exhibits(), &args.station_only);
    anyhow::ensure!(
        !exhibits.is_empty(),
        "no exhibit matches --station-only {}",
        args.station_only.join(",")
    );
    let (w, h) = args
        .station_size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?)))
        .ok_or_else(|| anyhow::anyhow!("--station-size wants WxH, for example 2560x1440"))?;
    let size = Size::new(w, h);
    let socket = resolve_socket_path(args.socket.clone());
    let script = match args.station_script.as_deref() {
        Some("demo") => Script::Demo,
        Some("live") => Script::Live,
        Some(s) => anyhow::bail!("--station-script is demo or live, not {s:?}"),
        // An offline render has no daemon to follow.
        None if args.offline.is_some() => Script::Demo,
        None if daemon_running(&socket) => Script::Live,
        None => Script::Demo,
    };
    let mut extra = args.motion.clone();
    if args.station_reduced {
        extra.push("reduced-motion".into());
    }
    tracing::info!(
        exhibits = exhibits.len(),
        ?script,
        "the motion station starts"
    );
    let app = Station::new(Setup {
        exhibits,
        cfg: cfg.clone(),
        theme: theme.clone(),
        stop_hint,
        fonts: fonts.clone(),
        extra,
        script,
        size,
        cols: args.station_cols,
        rows: args.station_rows,
        dwell: args.station_dwell,
        page: args.station_page,
    });
    let options = |inbox| Options {
        fonts: fonts.clone(),
        inbox,
        default_motion: Motion::Spring(Spring::SOFT),
        ..Options::default()
    };
    if let Some(dir) = &args.offline {
        let scenario = vo::Scenario {
            size,
            scale: 1.0,
            duration: app.dwell(),
            inputs: Vec::new(),
        };
        let n = vo::render_offline_with(app, &scenario, args.fps, dir, options(None))
            .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
        eprintln!(
            "voxtype-osd-vo: wrote {n} station frames to {}",
            dir.display()
        );
        return Ok(());
    }
    let inbox = if script == Script::Live {
        let shared: Shared = Arc::new((Mutex::new(Sink::default()), Condvar::new()));
        let reconnect = Duration::from_secs_f32(args.reconnect_secs.max(0.05));
        let (proxy, inbox) = vo::channel();
        shared.0.lock().unwrap().proxy = Some(proxy);
        {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("voxtype-osd-audio".into())
                .spawn(move || read_audio(shared, socket, reconnect))?;
        }
        std::thread::Builder::new()
            .name("voxtype-osd-files".into())
            .spawn(move || poll_files(shared))?;
        Some(inbox)
    } else {
        None
    };
    let spec = OverlaySpec {
        size,
        placement: Placement::Fullscreen,
        input: InputMode::None,
        app_id: "voxtype-osd-station".into(),
        ..OverlaySpec::default()
    };
    vo::run_overlay_with(app, spec, options(inbox)).map_err(|e| anyhow::anyhow!("station: {e}"))
}

/// Whether the daemon runs: its audio socket takes a connection.
pub(crate) fn daemon_running(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exhibit_has_an_id_a_name_and_a_file() {
        let all = exhibits();
        assert!(!all.is_empty());
        for e in &all {
            assert!(
                e.id.starts_with("MO-") && e.number() < 100,
                "bad id {:?}",
                e.id
            );
            assert!(!e.name.is_empty(), "{} has no name", e.id);
        }
        assert!(all.windows(2).all(|w| w[0].number() <= w[1].number()));
    }

    #[test]
    fn the_filter_takes_ids_numbers_and_names() {
        let all = exhibits();
        let first = all[0].clone();
        let by_id = filter(all.clone(), &[first.id.to_lowercase()]);
        assert!(by_id.iter().all(|e| e.id == first.id) && !by_id.is_empty());
        let by_n = filter(all.clone(), &[first.number().to_string()]);
        assert_eq!(by_n.len(), by_id.len());
        assert!(filter(all.clone(), &["MO-999".into()]).is_empty());
        assert_eq!(filter(all.clone(), &[]).len(), all.len());
    }

    #[test]
    fn a_tile_cycles_through_its_moments() {
        let e = Exhibit::new("MO-07", "x", OsdTheme::Swiss)
            .moments(&[Moment::Cancel, Moment::NoSpeech]);
        assert_eq!(e.moment_at(0), Moment::Cancel);
        assert_eq!(e.moment_at(1), Moment::NoSpeech);
        assert_eq!(e.moment_at(2), Moment::Cancel);
        assert_eq!(
            Exhibit::new("MO-07", "x", OsdTheme::Swiss).moment_at(3),
            Moment::Dictation
        );
    }
}
