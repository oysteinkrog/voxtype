//! Colors and fonts from the KDE Plasma color scheme, for the `swiss` OSD
//! theme.
//!
//! Plasma writes the active scheme to `~/.config/kdeglobals` as INI groups
//! such as `[Colors:View]` with `Key=R,G,B` values. Any missing file, group
//! or key falls back to a built-in dark scheme (Plasma's Breeze Dark).

use std::path::PathBuf;

use super::visual::Color;

/// Colors and font families the `swiss` theme draws with.
#[derive(Debug, Clone, PartialEq)]
pub struct PlasmaTheme {
    /// Panel background (`[Colors:View] BackgroundNormal`).
    pub background: Color,
    /// Transcript and value text (`ForegroundNormal`).
    pub foreground: Color,
    /// Labels, status text and quiet waveform ticks (`ForegroundInactive`).
    pub inactive: Color,
    /// The one accent color: `[General] AccentColor` when set, else the
    /// selection background, else `DecorationFocus`.
    pub accent: Color,
    /// 1 px border and hairlines, mixed from background and foreground.
    pub border: Color,
    /// UI font family (`[General] font`), if Plasma names one.
    pub font_family: Option<String>,
    /// Fixed-width font family (`[General] fixed`), if Plasma names one.
    pub mono_family: Option<String>,
}

impl Default for PlasmaTheme {
    fn default() -> Self {
        let background = rgb8(20, 22, 24);
        let foreground = rgb8(252, 252, 252);
        Self {
            background,
            foreground,
            inactive: rgb8(161, 169, 177),
            accent: rgb8(61, 174, 233),
            border: mix(background, foreground, BORDER_MIX),
            font_family: None,
            mono_family: None,
        }
    }
}

/// How far the border sits from the background toward the foreground.
const BORDER_MIX: f32 = 0.16;

fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

/// `~/.config/kdeglobals`, honoring `XDG_CONFIG_HOME`.
pub fn kdeglobals_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kdeglobals"))
}

/// Read the active Plasma scheme, or the built-in one if there is none.
pub fn load() -> PlasmaTheme {
    kdeglobals_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| parse(&s))
        .unwrap_or_default()
}

/// Parse the text of a kdeglobals file.
pub fn parse(content: &str) -> PlasmaTheme {
    let get = |group: &str, key: &str| lookup(content, group, key);
    let color = |group: &str, key: &str| get(group, key).and_then(|v| parse_rgb(&v));
    let fallback = PlasmaTheme::default();

    let background = color("Colors:View", "BackgroundNormal")
        .or_else(|| color("Colors:Window", "BackgroundNormal"))
        .unwrap_or(fallback.background);
    let foreground = color("Colors:View", "ForegroundNormal")
        .or_else(|| color("Colors:Window", "ForegroundNormal"))
        .unwrap_or(fallback.foreground);
    let inactive = color("Colors:View", "ForegroundInactive")
        .or_else(|| color("Colors:Window", "ForegroundInactive"))
        .unwrap_or(fallback.inactive);
    let accent = color("General", "AccentColor")
        .or_else(|| color("Colors:Selection", "BackgroundNormal"))
        .or_else(|| color("Colors:View", "DecorationFocus"))
        .unwrap_or(fallback.accent);

    PlasmaTheme {
        background,
        foreground,
        inactive,
        accent,
        border: mix(background, foreground, BORDER_MIX),
        font_family: get("General", "font").and_then(|v| font_family(&v)),
        mono_family: get("General", "fixed").and_then(|v| font_family(&v)),
    }
}

/// Value of `key` in `[group]`. Only the plain group counts: Plasma also
/// writes sub-groups such as `[Colors:Header][Inactive]`.
fn lookup(content: &str, group: &str, key: &str) -> Option<String> {
    let header = format!("[{group}]");
    let mut in_group = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == header;
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// "R,G,B" (an optional fourth alpha field is ignored).
fn parse_rgb(value: &str) -> Option<Color> {
    let mut parts = value.split(',').map(|p| p.trim().parse::<u8>());
    let (r, g, b) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    Some(rgb8(r, g, b))
}

/// Family name from a Qt font string such as "Noto Sans,10,-1,5,400,0,...".
fn font_family(value: &str) -> Option<String> {
    let family = value.split(',').next()?.trim();
    (!family.is_empty()).then(|| family.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[Colors:Header][Inactive]
BackgroundNormal=1,1,1

[Colors:Selection]
BackgroundNormal=61,174,233

[Colors:View]
BackgroundNormal=20,22,24
ForegroundInactive=161,169,177
ForegroundNormal=252,252,252

[General]
fixed=Hack,10,-1,5,400,0,0,0,0,0
font=Noto Sans,10,-1,5,400,0,0,0,0,0
";

    #[test]
    fn reads_view_colors_accent_and_fonts() {
        let t = parse(SAMPLE);
        assert_eq!(t.background, rgb8(20, 22, 24));
        assert_eq!(t.foreground, rgb8(252, 252, 252));
        assert_eq!(t.inactive, rgb8(161, 169, 177));
        assert_eq!(t.accent, rgb8(61, 174, 233));
        assert_eq!(t.font_family.as_deref(), Some("Noto Sans"));
        assert_eq!(t.mono_family.as_deref(), Some("Hack"));
    }

    #[test]
    fn accent_color_setting_wins_over_selection() {
        let t = parse(&format!("{SAMPLE}AccentColor=233,100,12\n"));
        assert_eq!(t.accent, rgb8(233, 100, 12));
    }

    #[test]
    fn falls_back_to_window_group_then_builtin() {
        let t = parse("[Colors:Window]\nBackgroundNormal=32,35,38\n");
        assert_eq!(t.background, rgb8(32, 35, 38));
        assert_eq!(t.foreground, PlasmaTheme::default().foreground);
        assert_eq!(t.font_family, None);
        assert_eq!(parse(""), PlasmaTheme::default());
    }

    #[test]
    fn ignores_malformed_values() {
        let t = parse("[Colors:View]\nBackgroundNormal=20,22\nForegroundNormal=a,b,c\n");
        assert_eq!(t.background, PlasmaTheme::default().background);
        assert_eq!(t.foreground, PlasmaTheme::default().foreground);
    }
}
