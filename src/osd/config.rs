//! `[osd]` configuration block.
//!
//! Parsed from the user's config file alongside the rest of the daemon
//! config; can be overridden via CLI flags or `VOXTYPE_OSD_*` env vars on
//! either OSD binary. The full config layering is wired up in Commit 6.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Position anchor for the OSD surface on the focused output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdPosition {
    #[default]
    BottomCenter,
    TopCenter,
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
}

/// How the centered anchors place the popup vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdAnchor {
    /// The popup's top edge sits at `top_margin` of the monitor height, and
    /// the popup grows downward as the text gets longer.
    #[default]
    Top,
    /// The newest text line, the one being read, sits at `reading_line` of
    /// the monitor height, and the popup grows upward as the text gets
    /// longer, so the line being read stays put.
    ReadingLine,
    /// The middle of the popup at its full height sits at `reading_line` of
    /// the monitor height, so every look is centred on the same spot.
    Center,
}

impl OsdAnchor {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "top" => Some(OsdAnchor::Top),
            "reading-line" => Some(OsdAnchor::ReadingLine),
            "center" => Some(OsdAnchor::Center),
            _ => None,
        }
    }
}

/// Selects which OSD frontend the `voxtype-osd` wrapper launches.
///
/// The wrapper treats this as a *preference*: if the chosen frontend's
/// binary isn't on PATH (e.g. the user built voxtype with only one of
/// `osd-gtk4`/`osd-native`/`osd-quickshell`), the wrapper falls back to
/// whichever it can find and logs a warning. Default is `Gtk4` because
/// GTK4 ships with most Hyprland setups already (Omarchy pulls it in via
/// swayosd, walker, etc.) so there's no extra runtime cost for the
/// typical user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdFrontend {
    #[default]
    Gtk4,
    Native,
    Quickshell,
    /// The Swiss popup on the vo GPU overlay framework (feature `osd-vo`).
    /// Opt-in only: no other preference falls back to it.
    Vo,
}

impl OsdFrontend {
    /// Name of the binary that implements this frontend, suitable for a
    /// PATH lookup or `Command::new`.
    pub fn binary_name(self) -> &'static str {
        match self {
            OsdFrontend::Gtk4 => "voxtype-osd-gtk4",
            OsdFrontend::Native => "voxtype-osd-native",
            OsdFrontend::Quickshell => "voxtype-osd-quickshell",
            OsdFrontend::Vo => "voxtype-osd-vo",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gtk4" | "gtk" => Some(OsdFrontend::Gtk4),
            "native" | "wgpu" | "egui" => Some(OsdFrontend::Native),
            "quickshell" | "qml" | "qs" => Some(OsdFrontend::Quickshell),
            "vo" => Some(OsdFrontend::Vo),
            _ => None,
        }
    }
}

/// Look of the gtk4 OSD panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdTheme {
    /// Filled waveform with a segmented peak meter.
    #[default]
    Default,
    /// Header row with elapsed time and word count, a filled gradient waveform,
    /// large transcript text and a status line. Colors and font come from
    /// the KDE Plasma color scheme (`~/.config/kdeglobals`).
    Swiss,
    /// A rich look that shows off the vo GPU overlay: glowing ribbon
    /// waveform, rim light, a tilted ring of dots and drifting orbs around
    /// the swiss popup's layout. Drawn by the vo frontend only; the gtk4
    /// frontend draws `swiss` for it.
    Showcase,
    /// The swiss popup's layout led by type and line: the frame draws
    /// itself on, the clock and word count roll their digits, the icon
    /// morphs from microphone to stop to check, and words rise into place.
    /// Drawn by the vo frontend only; the gtk4 frontend draws `swiss` for it.
    Type,
    /// An amber hologram panel: thin geometric frames that draw on, vertical
    /// micro-labels, a fine-line waveform and the transcript in a condensed
    /// face. Drawn by the vo frontend only; the gtk4 frontend draws `swiss`
    /// for it.
    Amber,
    /// A radial spectrum: bars round a microphone in a blue to violet ring
    /// over a soft bloom, and the transcript in a wide card below. Drawn by
    /// the vo frontend only; the gtk4 frontend draws `swiss` for it.
    Radial,
    /// Tron light lines: a black glass panel traced by cyan light lines that
    /// run round its border, the waveform as one luminous line, and the
    /// transcript in a thin geometric face. Drawn by the vo frontend only;
    /// the gtk4 frontend draws `swiss` for it.
    Tron,
    /// A green CRT terminal: a bowed screen with scanlines, a block meter,
    /// the transcript in a monospaced face typed in behind a block caret, and
    /// an italic status line. Drawn by the vo frontend only; the gtk4
    /// frontend draws `swiss` for it.
    Phosphor,
    /// A mission-control telemetry panel: tiles for the model, VRAM,
    /// latency, microphone and elapsed time, a large transcript tile with
    /// corner brackets, a waveform strip with a LIVE meter, orange and white
    /// on dark slate. Drawn by the vo frontend only; the gtk4 frontend draws
    /// `swiss` for it.
    Mission,
    /// A command palette: a crisp dark card with chips for the engine, the
    /// output and the latency, the transcript as input text with a caret,
    /// unsure words softly underlined and key badges in the status. Drawn by
    /// the vo frontend only; the gtk4 frontend draws `swiss` for it.
    Palette,
    /// A dot-matrix look: white dots on black, a waveform built from dots,
    /// the status in a dot-matrix face, the transcript in a clean grotesk and
    /// one red dot while recording. Drawn by the vo frontend only; the gtk4
    /// frontend draws `swiss` for it.
    Dotmatrix,
    /// Swiss type meets sci-fi: off-black and off-white with one orange
    /// accent, huge grotesk words, a hairline waveform of ticks with an
    /// orange centre, and small mono numerals that roll. Drawn by the vo
    /// frontend only; the gtk4 frontend draws `swiss` for it.
    Swissfi,
    /// A green oscilloscope instrument: a vector trace over a graticule, a
    /// readout column with level, time and words per minute, and the words
    /// in a monospaced readout panel with a block caret. Drawn by the vo
    /// frontend only; the gtk4 frontend draws `swiss` for it.
    Scope,
    /// Cinema subtitles: the words in two large centred lines on a soft
    /// dark backdrop, the newest word in accent blue, lines that roll up, and
    /// a small red dot that pulses while the mic is live. Drawn by the vo
    /// frontend only; the gtk4 frontend draws `swiss` for it.
    Captions,
    /// A teleprompter in a dark navy glass card: large centred words, older
    /// lines that roll up, shrink and fade, the newest word lit in a cyan to
    /// violet gradient with an underline that hops from word to word, and a
    /// thin mirrored waveform. Drawn by the vo frontend only; the gtk4
    /// frontend draws `swiss` for it.
    Karaoke,
    /// A black capsule in the style of a phone's dynamic island: it opens
    /// as a dot, springs out to a pill with a red recording dot, a clock, a
    /// blue to violet bar waveform and up to three lines of words, and grows
    /// into a card when the words need more room. Drawn by the vo frontend
    /// only; the gtk4 frontend draws `swiss` for it.
    Island,
    /// A holographic HUD in cyan: arcs, ticks and thin ring segments that
    /// turn round a radial waveform with the microphone in the middle, small
    /// level readouts, and the transcript centred in a chamfered frame. Drawn
    /// by the vo frontend only; the gtk4 frontend draws `swiss` for it.
    Jarvis,
    /// Mock-up 09, particle words: a navy glass card with a swirl of dotted
    /// light round the microphone that follows the voice, and large centred
    /// words that form out of particles. Drawn by the vo frontend only; the
    /// gtk4 frontend draws `swiss` for it.
    Particles,
    /// Retro synthwave neon: a magenta to cyan neon tube round a dark card,
    /// a chrome title that names the state, a neon bar waveform over a
    /// scrolling perspective grid, and white centred words. Drawn by the vo
    /// frontend only; the gtk4 frontend draws `swiss` for it.
    Synthwave,
    /// An anime mecha cockpit: an orange angular frame with hexagon plates,
    /// a Japanese and English title, hazard stripes, a big mirrored bar
    /// waveform, a level meter and the transcript in heavy condensed type.
    /// Drawn by the vo frontend only; the gtk4 frontend draws `swiss` for it.
    Mecha,
    /// A cyberpunk HUD: a chamfered black panel in a yellow and red frame, a
    /// yellow tag that types "VOICE LINK ACTIVE", hazard slashes, yellow and
    /// red bars between brackets, and bold white words. Glitch slices fire on
    /// a revised word or a change of state. Drawn by the vo frontend only;
    /// the gtk4 frontend draws `swiss` for it.
    Cyberpunk,
}

impl OsdTheme {
    /// Every look the vo frontend draws, in declaration order. `Default`
    /// is left out: vo draws it as `Swiss`.
    pub const VO_LOOKS: [OsdTheme; 20] = [
        OsdTheme::Swiss,
        OsdTheme::Showcase,
        OsdTheme::Type,
        OsdTheme::Amber,
        OsdTheme::Radial,
        OsdTheme::Tron,
        OsdTheme::Phosphor,
        OsdTheme::Mission,
        OsdTheme::Palette,
        OsdTheme::Dotmatrix,
        OsdTheme::Swissfi,
        OsdTheme::Scope,
        OsdTheme::Captions,
        OsdTheme::Karaoke,
        OsdTheme::Island,
        OsdTheme::Jarvis,
        OsdTheme::Particles,
        OsdTheme::Synthwave,
        OsdTheme::Mecha,
        OsdTheme::Cyberpunk,
    ];

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" => Some(OsdTheme::Default),
            "swiss" => Some(OsdTheme::Swiss),
            "showcase" => Some(OsdTheme::Showcase),
            "type" => Some(OsdTheme::Type),
            "amber" => Some(OsdTheme::Amber),
            "radial" => Some(OsdTheme::Radial),
            "tron" => Some(OsdTheme::Tron),
            "phosphor" => Some(OsdTheme::Phosphor),
            "mission" => Some(OsdTheme::Mission),
            "palette" => Some(OsdTheme::Palette),
            "dotmatrix" => Some(OsdTheme::Dotmatrix),
            "swissfi" => Some(OsdTheme::Swissfi),
            "scope" => Some(OsdTheme::Scope),
            "captions" => Some(OsdTheme::Captions),
            "karaoke" => Some(OsdTheme::Karaoke),
            "island" => Some(OsdTheme::Island),
            "jarvis" => Some(OsdTheme::Jarvis),
            "particles" => Some(OsdTheme::Particles),
            "cyberpunk" => Some(OsdTheme::Cyberpunk),
            "synthwave" => Some(OsdTheme::Synthwave),
            "mecha" => Some(OsdTheme::Mecha),
            _ => None,
        }
    }
}

/// Palette source for Quickshell OSD recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdPaletteSource {
    /// Resolve semantic colors from the active Omarchy theme.
    #[default]
    Omarchy,
    /// Use VoxType's built-in fallback palette.
    Fallback,
    /// Use colors provided by the selected OSD package.
    Package,
    /// Use literal colors from the visual recipe.
    Custom,
}

/// Layout preset for the Quickshell OSD host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdLayout {
    #[default]
    Compact,
    Wide,
    Minimal,
    Tile,
    Orb,
    Custom,
}

/// Declarative layer kind for no-code Quickshell OSD recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OsdLayerKind {
    Shadow,
    Background,
    Waveform,
    Bars,
    Pulse,
    Ring,
    Meter,
    Icon,
    Label,
}

/// One declarative visual layer in the Quickshell renderer.
///
/// Tunables are `Option` and skipped from the runtime JSON when unset, so
/// the QML renderer can distinguish "not configured" (apply the layer
/// type's own default) from an explicit value, including explicit zeros.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdLayerConfig {
    /// Layer type. Serialized as `type` in TOML manifests/recipes.
    #[serde(rename = "type")]
    pub kind: OsdLayerKind,
    /// Input signal: peak, rms, vad, state, or none.
    pub source: String,
    /// Semantic color role or literal color. Unset lets each layer type
    /// pick its own default (accent for most, black for shadow).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Secondary color role or literal color for gradients/ticks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_color: Option<String>,
    /// Z-order. Lower layers render first.
    pub order: i32,
    /// Normalized or pixel position hint consumed by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    /// Normalized or pixel position hint consumed by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    /// Width hint. Values <= 1.0 are treated as a fraction by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Height hint. Values <= 1.0 are treated as a fraction by QML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// Audio visual gain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    /// Opacity, 0.0..=1.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// Corner/ring radius hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Initial size multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_scale: Option<f32>,
    /// Audio response multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_scale: Option<f32>,
    /// Audio response curve; values below 1.0 make low/mid signals more visible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_curve: Option<f32>,
    /// Idle breathing multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breath_scale: Option<f32>,
    /// Maximum size multiplier for scalable shapes such as rings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_scale: Option<f32>,
    /// Mirror bars/waveform around the center line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<bool>,
    /// Animation speed multiplier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
}

impl Default for OsdLayerConfig {
    fn default() -> Self {
        Self {
            kind: OsdLayerKind::Waveform,
            source: "peak".to_string(),
            color: None,
            secondary_color: None,
            order: 0,
            x: None,
            y: None,
            width: None,
            height: None,
            gain: None,
            opacity: None,
            radius: None,
            base_scale: None,
            response_scale: None,
            response_curve: None,
            breath_scale: None,
            max_scale: None,
            mirror: None,
            speed: None,
        }
    }
}

/// Declarative visual recipe for the Quickshell OSD.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdVisualConfig {
    pub layers: Vec<OsdLayerConfig>,
}

impl Default for OsdVisualConfig {
    fn default() -> Self {
        Self {
            layers: default_visual_layers(),
        }
    }
}

fn default_visual_layers() -> Vec<OsdLayerConfig> {
    vec![
        OsdLayerConfig {
            kind: OsdLayerKind::Waveform,
            source: "peak".to_string(),
            color: Some("accent".to_string()),
            order: 10,
            height: Some(0.82),
            mirror: Some(true),
            ..OsdLayerConfig::default()
        },
        OsdLayerConfig {
            kind: OsdLayerKind::Meter,
            source: "peak".to_string(),
            color: Some("success".to_string()),
            secondary_color: Some("foreground".to_string()),
            order: 20,
            y: Some(0.86),
            height: Some(0.14),
            ..OsdLayerConfig::default()
        },
    ]
}

/// Quickshell host-frame styling for no-code recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdFrameConfig {
    /// Background color role, literal color, or `none`.
    pub background: String,
    /// Border color role, `state`, literal color, or `none`.
    pub border: String,
    /// Whether the voice-reactive soft frame glow is rendered.
    pub glow: bool,
    /// Whether the orb layout renders its extra outline halo.
    pub halo: bool,
}

impl Default for OsdFrameConfig {
    fn default() -> Self {
        Self {
            background: "background".to_string(),
            border: "state".to_string(),
            glow: true,
            halo: true,
        }
    }
}

/// Manifest for a shareable Quickshell OSD customization package.
///
/// `palette`, `layout`, `frame`, and `visual` are `Option` on purpose:
/// a manifest only overrides the user's `[osd]` config for fields it
/// explicitly sets. An unset field must never reset the user's choice
/// back to a built-in default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdPackageManifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub compatibility: Option<String>,
    pub palette: Option<OsdPaletteSource>,
    pub layout: Option<OsdLayout>,
    pub colors: BTreeMap<String, String>,
    pub qml_entry: Option<PathBuf>,
    pub frame: Option<OsdFrameConfig>,
    pub visual: Option<OsdVisualConfig>,
}

impl Default for OsdPackageManifest {
    fn default() -> Self {
        Self {
            name: "custom".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            compatibility: None,
            palette: None,
            layout: None,
            colors: BTreeMap::new(),
            qml_entry: None,
            frame: None,
            visual: None,
        }
    }
}

/// Which motion features the vo popup turns on (`[osd] motion`).
///
/// In the config file this is `"look"` (the default: each look's own set),
/// `"none"` (no features unless asked for) or a list of feature names. A
/// list with any plain name replaces the look's set; `no-<name>` takes a
/// name out; a list of only `no-` entries is the look's set minus those.
/// Which names count as features is the popup's business: an unknown name
/// is kept here and ignored there, never an error.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OsdMotion {
    /// The look's own motion set.
    #[default]
    Look,
    /// No motion features unless a name asks for one.
    None,
    /// These names, as written (trimmed, empty ones dropped).
    Names(Vec<String>),
}

impl OsdMotion {
    /// From names as the CLI or the environment give them: one `look` or
    /// `none` is that mode, anything else is a list.
    pub fn from_names<I, S>(names: I) -> OsdMotion
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let names: Vec<String> = names
            .into_iter()
            .flat_map(|n| {
                n.as_ref()
                    .split(',')
                    .map(|p| p.trim().to_ascii_lowercase())
                    .collect::<Vec<_>>()
            })
            .filter(|n| !n.is_empty())
            .collect();
        match names.as_slice() {
            [] => OsdMotion::Look,
            [one] if one == "look" => OsdMotion::Look,
            [one] if one == "none" => OsdMotion::None,
            _ => OsdMotion::Names(names),
        }
    }

    /// The setting in force: `over` (the CLI flag, or else the environment
    /// variable, as clap already resolved them) when it names anything,
    /// otherwise the config file's.
    pub fn layered(over: &[String], config: &OsdMotion) -> OsdMotion {
        if over.iter().all(|n| n.trim().is_empty()) {
            config.clone()
        } else {
            OsdMotion::from_names(over)
        }
    }
}

impl Serialize for OsdMotion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            OsdMotion::Look => s.serialize_str("look"),
            OsdMotion::None => s.serialize_str("none"),
            OsdMotion::Names(n) => n.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for OsdMotion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            One(String),
            Many(Vec<String>),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::One(s) => OsdMotion::from_names([s]),
            // An empty list asks for nothing: no defaults either.
            Raw::Many(v) if v.iter().all(|n| n.trim().is_empty()) => OsdMotion::None,
            Raw::Many(v) => OsdMotion::from_names(v),
        })
    }
}

/// All user-facing OSD options. Defaults match BRIEF.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    /// Run the OSD at all. When `false`, both binaries exit immediately.
    pub enabled: bool,
    /// Surface width in physical pixels.
    pub width_px: u32,
    /// Surface height in physical pixels.
    pub height_px: u32,
    /// Anchor on the focused output.
    pub position: OsdPosition,
    /// Margin from the screen edge in physical pixels. Used for corner
    /// anchors (`top-left`, `bottom-right`, etc.) and as a fallback for
    /// centered anchors. Centered anchors (`bottom-center`, `top-center`)
    /// prefer `top_margin` (fractional) so v0.7.0 ships swayosd-aligned
    /// vertical positioning out of the box; this field still sets the
    /// horizontal margin for corner anchors.
    pub margin_px: u32,
    /// Vertical position of the OSD's top edge as a fraction of the
    /// monitor's height, mirroring `swayosd-server --top-margin`. Default
    /// 0.85 puts the panel just above the bottom of the screen, matching
    /// the swayosd default so the voxtype OSD lands in the same band as
    /// volume/brightness/media-key feedback users are used to. Only
    /// applied when `position` is `bottom-center` or `top-center` —
    /// corner anchors keep using `margin_px`.
    pub top_margin: f32,
    /// What the centered anchors place: the popup's top edge (`top`, the
    /// default, at `top_margin`) or its newest text line (`reading-line`,
    /// at `reading_line`). Corner anchors ignore it.
    pub anchor: OsdAnchor,
    /// Height of the newest text line's centre (`anchor = "reading-line"`)
    /// or of the popup's middle (`anchor = "center"`) as a fraction of the
    /// monitor's height. 0.5 is the middle of the screen.
    pub reading_line: f32,
    /// Font size of the live preview text in points (gtk4 OSD). `None`
    /// keeps the GTK theme's size.
    pub preview_font_pt: Option<f32>,
    /// Background opacity, 0.0..=1.0.
    pub opacity: f32,
    /// Look of the gtk4 OSD panel. `default` keeps the original look.
    pub theme: OsdTheme,
    /// Fade new live words in (vo OSD only). Off by default: words appear
    /// at once, as in the gtk4 OSD.
    pub word_fade: bool,
    /// Show the active look's place in the look chord's list and its name,
    /// such as "3/20 Type", above the vo popup's top-left corner. Off by
    /// default.
    pub look_badge: bool,
    /// Run the vo popup's threads at real-time priority (`SCHED_RR` 1)
    /// while it shows, so a busy machine does not make it stutter. Needs
    /// `RLIMIT_RTPRIO` (the `audio` or `realtime` group); without it the
    /// popup takes the lowest nice value it may, down to -10. On by default.
    pub realtime: bool,
    /// Pause a live preview dictation and fix its text by keyboard in the
    /// vo popup: a single tap of `[hotkey] edit_key`, or `voxtype record
    /// edit`. Off by default.
    pub keyboard_edit: bool,
    /// The vo popup marks a word as unsure (Type's wavy line, Palette's
    /// wave, Captions' thin line) when the engine's confidence in it is
    /// below this, 0 to 1. Default [`DOUBT_BELOW`].
    pub doubt_below: f32,
    /// Motion features of the vo popup: `"look"` (each look's own set),
    /// `"none"`, or a list of names (see [`OsdMotion`]).
    pub motion: OsdMotion,
    /// Visible waveform window in seconds (3.0 per BRIEF).
    pub waveform_window_secs: f32,
    /// Held-peak decay rate in dB/sec (6.0 per BRIEF).
    pub peak_decay_db_per_sec: f32,
    /// Visual gain applied to audio samples before drawing the waveform.
    /// Mic-level voice typically peaks at ~0.1..=0.3 of full-scale; gain
    /// scales that up so the envelope fills the available height. 10.0 is
    /// the default; reduce for hot mics, increase for quiet sources.
    pub waveform_gain: f32,
    /// Which OSD frontend the `voxtype-osd` wrapper launches. Defaults to
    /// `Gtk4` since GTK4 ships with most Hyprland setups already.
    pub frontend: OsdFrontend,
    /// Quickshell style name, package name, or package path.
    pub style: String,
    /// Palette source for Quickshell OSD recipes.
    ///
    /// `None` means the selected package manifest may choose a palette.
    /// `Some(Omarchy)` is an explicit user choice and overrides manifests.
    pub palette: Option<OsdPaletteSource>,
    /// Layout preset for the Quickshell OSD host.
    pub layout: OsdLayout,
    /// Explicit third-party package path. QML code in this path is trusted.
    pub plugin_path: Option<PathBuf>,
    /// Quickshell host-frame styling for no-code recipes.
    pub frame: OsdFrameConfig,
    /// Declarative no-code visual recipe for the Quickshell OSD.
    pub visual: OsdVisualConfig,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width_px: 400,
            height_px: 48,
            position: OsdPosition::BottomCenter,
            margin_px: 24,
            top_margin: 0.85,
            anchor: OsdAnchor::Top,
            reading_line: 0.5,
            preview_font_pt: None,
            opacity: 0.95,
            theme: OsdTheme::Default,
            word_fade: false,
            look_badge: false,
            realtime: true,
            keyboard_edit: false,
            doubt_below: DOUBT_BELOW,
            motion: OsdMotion::Look,
            waveform_window_secs: 3.0,
            peak_decay_db_per_sec: 6.0,
            waveform_gain: 10.0,
            frontend: OsdFrontend::default(),
            style: "default".to_string(),
            palette: None,
            layout: OsdLayout::default(),
            plugin_path: None,
            frame: OsdFrameConfig::default(),
            visual: OsdVisualConfig::default(),
        }
    }
}

/// The default `doubt_below`. Measured on 90 minutes of real dictation
/// with the live model (parakeet-unified-en-0.6b-int8, 11881 words, ASR-01,
/// Oct 2026), a word's confidence (its lowest token probability) has these
/// quantiles: 0.1 % 0.29, 1 % 0.45, 5 % 0.55, 10 % 0.63, 25 % 0.87, median
/// 0.99. The old 0.6 marked 8.3 % of words and some word in 77 % of
/// dictations; 0.45 marks about 1 % of words, the ones most likely wrong.
pub const DOUBT_BELOW: f32 = 0.45;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubt_below_defaults_to_the_measured_one_percent() {
        assert_eq!(OsdConfig::default().doubt_below, DOUBT_BELOW);
        let old: OsdConfig = toml::from_str("look_badge = true").unwrap();
        assert_eq!(old.doubt_below, 0.45);
        let set: OsdConfig = toml::from_str("doubt_below = 0.3").unwrap();
        assert_eq!(set.doubt_below, 0.3);
    }

    #[test]
    fn keyboard_edit_is_off_unless_set() {
        assert!(!OsdConfig::default().keyboard_edit);
        let old: OsdConfig = toml::from_str("look_badge = true").unwrap();
        assert!(!old.keyboard_edit);
        let on: OsdConfig = toml::from_str("keyboard_edit = true").unwrap();
        assert!(on.keyboard_edit);
    }

    #[test]
    fn defaults_match_brief() {
        let c = OsdConfig::default();
        assert!(c.enabled);
        assert_eq!(c.width_px, 400);
        assert_eq!(c.height_px, 48);
        assert_eq!(c.position, OsdPosition::BottomCenter);
        assert_eq!(c.margin_px, 24);
        assert!((c.opacity - 0.95).abs() < 1e-6);
        assert!((c.waveform_window_secs - 3.0).abs() < 1e-6);
        assert!((c.peak_decay_db_per_sec - 6.0).abs() < 1e-6);
        assert!((c.waveform_gain - 10.0).abs() < 1e-6);
        assert_eq!(c.style, "default");
        assert_eq!(c.theme, OsdTheme::Default);
        assert_eq!(c.palette, None);
        assert_eq!(c.layout, OsdLayout::Compact);
        assert!(c.plugin_path.is_none());
        assert_eq!(c.frame.background, "background");
        assert_eq!(c.frame.border, "state");
        assert!(c.frame.glow);
        assert!(c.frame.halo);
        assert_eq!(c.visual.layers.len(), 2);
    }

    #[test]
    fn position_serde_kebab_case() {
        let v: OsdPosition = serde_json::from_str("\"bottom-center\"").unwrap();
        assert_eq!(v, OsdPosition::BottomCenter);
        let v: OsdPosition = serde_json::from_str("\"top-right\"").unwrap();
        assert_eq!(v, OsdPosition::TopRight);
    }

    #[test]
    fn frontend_default_is_gtk4() {
        assert_eq!(OsdFrontend::default(), OsdFrontend::Gtk4);
        assert_eq!(OsdConfig::default().frontend, OsdFrontend::Gtk4);
    }

    #[test]
    fn frontend_binary_names() {
        assert_eq!(OsdFrontend::Gtk4.binary_name(), "voxtype-osd-gtk4");
        assert_eq!(OsdFrontend::Native.binary_name(), "voxtype-osd-native");
        assert_eq!(
            OsdFrontend::Quickshell.binary_name(),
            "voxtype-osd-quickshell"
        );
    }

    #[test]
    fn frontend_parse_str_accepts_aliases() {
        assert_eq!(OsdFrontend::parse_str("gtk4"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("GTK"), Some(OsdFrontend::Gtk4));
        assert_eq!(OsdFrontend::parse_str("native"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("wgpu"), Some(OsdFrontend::Native));
        assert_eq!(OsdFrontend::parse_str("egui"), Some(OsdFrontend::Native));
        assert_eq!(
            OsdFrontend::parse_str("quickshell"),
            Some(OsdFrontend::Quickshell)
        );
        assert_eq!(OsdFrontend::parse_str("qml"), Some(OsdFrontend::Quickshell));
        assert_eq!(OsdFrontend::parse_str("QS"), Some(OsdFrontend::Quickshell));
        assert_eq!(OsdFrontend::parse_str("vo"), Some(OsdFrontend::Vo));
        assert_eq!(OsdFrontend::Vo.binary_name(), "voxtype-osd-vo");
        assert_eq!(OsdFrontend::parse_str("nope"), None);
    }

    #[test]
    fn frontend_serde_kebab_case() {
        let v: OsdFrontend = serde_json::from_str("\"gtk4\"").unwrap();
        assert_eq!(v, OsdFrontend::Gtk4);
        let v: OsdFrontend = serde_json::from_str("\"native\"").unwrap();
        assert_eq!(v, OsdFrontend::Native);
        let v: OsdFrontend = serde_json::from_str("\"quickshell\"").unwrap();
        assert_eq!(v, OsdFrontend::Quickshell);
    }

    #[test]
    fn theme_parses_from_toml_and_str() {
        let c: OsdConfig = toml::from_str("theme = \"swiss\"\n").unwrap();
        assert_eq!(c.theme, OsdTheme::Swiss);
        assert_eq!(OsdTheme::parse_str(" Swiss "), Some(OsdTheme::Swiss));
        assert_eq!(OsdTheme::parse_str("default"), Some(OsdTheme::Default));
        assert_eq!(OsdTheme::parse_str("glass"), None);
    }

    #[test]
    fn motion_defaults_to_the_looks_set() {
        assert_eq!(OsdConfig::default().motion, OsdMotion::Look);
        let c: OsdConfig = toml::from_str("width_px = 800\n").unwrap();
        assert_eq!(c.motion, OsdMotion::Look);
    }

    #[test]
    fn motion_parses_the_string_forms() {
        let c: OsdConfig = toml::from_str("motion = \"look\"\n").unwrap();
        assert_eq!(c.motion, OsdMotion::Look);
        let c: OsdConfig = toml::from_str("motion = \"none\"\n").unwrap();
        assert_eq!(c.motion, OsdMotion::None);
        let c: OsdConfig = toml::from_str("motion = \" None \"\n").unwrap();
        assert_eq!(c.motion, OsdMotion::None);
        let c: OsdConfig = toml::from_str("motion = \"onset-ink\"\n").unwrap();
        assert_eq!(c.motion, OsdMotion::Names(vec!["onset-ink".into()]));
        let c: OsdConfig = toml::from_str("motion = \"onset-ink, rewind\"\n").unwrap();
        assert_eq!(
            c.motion,
            OsdMotion::Names(vec!["onset-ink".into(), "rewind".into()])
        );
    }

    #[test]
    fn motion_parses_the_list_form() {
        let c: OsdConfig = toml::from_str("motion = [\"no-fold-under\"]\n").unwrap();
        assert_eq!(c.motion, OsdMotion::Names(vec!["no-fold-under".into()]));
        let c: OsdConfig =
            toml::from_str("motion = [\"onset-ink\", \"no-rewind\"]\n").unwrap();
        assert_eq!(
            c.motion,
            OsdMotion::Names(vec!["onset-ink".into(), "no-rewind".into()])
        );
        let c: OsdConfig = toml::from_str("motion = [\"none\"]\n").unwrap();
        assert_eq!(c.motion, OsdMotion::None);
        let c: OsdConfig = toml::from_str("motion = []\n").unwrap();
        assert_eq!(c.motion, OsdMotion::None);
    }

    #[test]
    fn motion_keeps_unknown_names_without_an_error() {
        let c: OsdConfig =
            toml::from_str("motion = [\"no-such-thing\", \"warp-drive\"]\nwidth_px = 500\n")
                .unwrap();
        assert_eq!(c.width_px, 500);
        assert_eq!(
            c.motion,
            OsdMotion::Names(vec!["no-such-thing".into(), "warp-drive".into()])
        );
    }

    #[test]
    fn motion_round_trips_through_toml() {
        for m in [
            OsdMotion::Look,
            OsdMotion::None,
            OsdMotion::Names(vec!["onset-ink".into(), "no-fold-under".into()]),
        ] {
            let c = OsdConfig {
                motion: m.clone(),
                ..OsdConfig::default()
            };
            let back: OsdConfig = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
            assert_eq!(back.motion, m);
        }
    }

    #[test]
    fn motion_from_the_cli_or_env_wins_over_the_config() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let config = OsdMotion::Names(names(&["no-fold-under"]));
        // Nothing on the CLI or in the environment: the config holds.
        assert_eq!(OsdMotion::layered(&[], &config), config);
        assert_eq!(OsdMotion::layered(&names(&[""]), &config), config);
        // `--motion none` works over a config list.
        assert_eq!(
            OsdMotion::layered(&names(&["none"]), &config),
            OsdMotion::None
        );
        assert_eq!(
            OsdMotion::layered(&names(&["look"]), &OsdMotion::None),
            OsdMotion::Look
        );
        assert_eq!(
            OsdMotion::layered(&names(&["onset-ink"]), &OsdMotion::Look),
            OsdMotion::Names(names(&["onset-ink"]))
        );
        // A comma list in one value, as the environment variable gives it.
        assert_eq!(
            OsdMotion::from_names(["onset-ink,no-rewind"]),
            OsdMotion::Names(names(&["onset-ink", "no-rewind"]))
        );
    }

    #[test]
    fn config_partial_toml_uses_defaults() {
        let toml_src = "width_px = 800\n";
        let c: OsdConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(c.width_px, 800);
        // All other fields default
        assert_eq!(c.height_px, 48);
        assert!(c.enabled);
    }

    #[test]
    fn visual_layer_serde_uses_type_key() {
        let toml_src = r#"
            [[layers]]
            type = "bars"
            source = "rms"
            color = "accent"
            order = 5
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers.len(), 1);
        assert_eq!(v.layers[0].kind, OsdLayerKind::Bars);
        assert_eq!(v.layers[0].source, "rms");
        assert_eq!(v.layers[0].order, 5);
    }

    #[test]
    fn visual_layer_accepts_shadow_kind() {
        let toml_src = r#"
            [[layers]]
            type = "shadow"
            source = "peak"
            opacity = 0.8
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers.len(), 1);
        assert_eq!(v.layers[0].kind, OsdLayerKind::Shadow);
        assert_eq!(v.layers[0].source, "peak");
        assert!((v.layers[0].opacity.unwrap() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn visual_layer_distinguishes_explicit_zero_from_unset() {
        let toml_src = r#"
            [[layers]]
            type = "pulse"
            gain = 0.0
            radius = 0.0
        "#;
        let v: OsdVisualConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(v.layers[0].gain, Some(0.0));
        assert_eq!(v.layers[0].radius, Some(0.0));
        assert_eq!(v.layers[0].speed, None);

        // Unset tunables must be absent from the runtime JSON so QML can
        // apply per-layer-type defaults without falsy-zero confusion.
        let json = serde_json::to_value(&v.layers[0]).unwrap();
        assert!(json.get("speed").is_none());
        assert!(json.get("opacity").is_none());
        assert_eq!(json["gain"], 0.0);
    }

    #[test]
    fn frame_config_accepts_none_values() {
        let toml_src = r#"
            [frame]
            background = "none"
            border = "none"
            glow = false
            halo = false
        "#;
        let c: OsdConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(c.frame.background, "none");
        assert_eq!(c.frame.border, "none");
        assert!(!c.frame.glow);
        assert!(!c.frame.halo);
    }

    #[test]
    fn example_recipes_parse_with_current_schema() {
        #[derive(serde::Deserialize)]
        struct Doc {
            osd: OsdConfig,
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/osd-recipes");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "toml") {
                let content = std::fs::read_to_string(&path).unwrap();
                let doc: Doc = toml::from_str(&content)
                    .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
                assert_eq!(
                    doc.osd.frontend,
                    OsdFrontend::Quickshell,
                    "{}",
                    path.display()
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 4,
            "expected the showcase recipes, found {checked}"
        );
    }

    #[test]
    fn package_manifest_leaves_unset_fields_to_user_config() {
        let manifest: OsdPackageManifest = toml::from_str(
            r#"
                name = "bars-plus"
                version = "1.0.0"
            "#,
        )
        .unwrap();
        assert_eq!(manifest.name, "bars-plus");
        assert_eq!(manifest.palette, None);
        assert_eq!(manifest.layout, None);
        assert!(manifest.colors.is_empty());
        assert!(manifest.frame.is_none());
        assert!(manifest.visual.is_none());
    }
}
