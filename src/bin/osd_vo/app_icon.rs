//! The icon of the app the text went to, for the delivery receipt (EG-24).
//!
//! The daemon names the focused app (`src/osd/target.rs`): a Wayland app id such as
//! `org.kde.konsole`, or an X11 window class. The icon is found the way a desktop finds it:
//!
//! 1. A desktop file: `<id>.desktop` in `applications/` of each XDG data dir, or else one
//!    whose `StartupWMClass` or file name matches the id, ignoring case. Its `Icon=` names
//!    the icon. Without a desktop file, the id itself is tried as an icon name.
//! 2. The icon theme: KDE's (`[Icons] Theme` in kdeglobals) and the themes it inherits, then
//!    hicolor, in each data dir's `icons/`. Scalable first, then the largest size, then
//!    `pixmaps/`. An `Icon=` that is a full path is used as it is.
//!
//! The lookup reads files, so it runs on a thread of its own the first time an app is
//! named, and decodes the icon into the shared image cache there. Until it is done the
//! receipt shows no icon, and the first frame never waits for it. A failed lookup is
//! logged once, at debug.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use vo::scene::geom::Size;
use vo::scene::image::ImageSource;

/// Sizes tried after scalable, largest first.
const SIZES: [u32; 10] = [512, 256, 128, 96, 64, 48, 32, 24, 22, 16];

/// The size the lookup thread decodes at, in logical units, for a 1x surface.
const WARM_SIZE: f32 = 14.0;

enum Slot {
    Pending,
    Found(Arc<ImageSource>),
    Missing,
}

fn icons() -> &'static Mutex<HashMap<String, Slot>> {
    static ICONS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    ICONS.get_or_init(Default::default)
}

/// Starts looking up `app`'s icon on a thread of its own, once per app.
pub(crate) fn request(app: &str) {
    {
        let mut map = icons().lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(app) {
            return;
        }
        map.insert(app.to_string(), Slot::Pending);
    }
    let app = app.to_string();
    let spawned = std::thread::Builder::new().name("vo-app-icon".into()).spawn(move || {
        let slot = match lookup(&app, &data_dirs(), &themes()) {
            Some(path) => {
                let src = ImageSource::file(&path);
                match src.warm(Size::new(WARM_SIZE, WARM_SIZE), 1.0) {
                    Ok(_) => Slot::Found(src),
                    Err(e) => {
                        tracing::debug!(%app, path = %path.display(), %e, "the app icon does not decode");
                        Slot::Missing
                    }
                }
            }
            None => {
                tracing::debug!(%app, "no icon found for the app");
                Slot::Missing
            }
        };
        icons().lock().unwrap_or_else(|e| e.into_inner()).insert(app, slot);
    });
    if spawned.is_err() {
        tracing::debug!("cannot start the app icon lookup");
    }
}

/// `app`'s icon, once the lookup found it.
pub(crate) fn get(app: &str) -> Option<Arc<ImageSource>> {
    match icons().lock().unwrap_or_else(|e| e.into_inner()).get(app)? {
        Slot::Found(src) => Some(src.clone()),
        Slot::Pending | Slot::Missing => None,
    }
}

/// The XDG data dirs: `XDG_DATA_HOME` (or `~/.local/share`), then `XDG_DATA_DIRS`.
fn data_dirs() -> Vec<PathBuf> {
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs::data_dir()
        .into_iter()
        .chain(std::env::split_paths(&system))
        .collect()
}

/// KDE's icon theme from kdeglobals, when set.
fn themes() -> Vec<String> {
    dirs::config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
        .and_then(|ini| ini_value(&ini, "Icons", "Theme"))
        .into_iter()
        .collect()
}

/// The icon file for `app`, looked up in `data` dirs with `themes` before hicolor.
pub(crate) fn lookup(app: &str, data: &[PathBuf], themes: &[String]) -> Option<PathBuf> {
    let icon = desktop_icon(app, data).unwrap_or_else(|| app.to_string());
    let path = Path::new(&icon);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    theme_icon(&icon, data, themes)
}

/// `Icon=` of the desktop file that belongs to `app`.
fn desktop_icon(app: &str, data: &[PathBuf]) -> Option<String> {
    let lower = app.to_lowercase();
    let exact = data.iter().find_map(|d| {
        [app, lower.as_str()]
            .iter()
            .map(|id| d.join("applications").join(format!("{id}.desktop")))
            .find_map(|p| std::fs::read_to_string(p).ok())
    });
    let text = exact.or_else(|| {
        data.iter().find_map(|d| {
            let dir = std::fs::read_dir(d.join("applications")).ok()?;
            let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
            files.sort();
            files
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
                .find_map(|p| {
                    let text = std::fs::read_to_string(&p).ok()?;
                    let class = ini_value(&text, "Desktop Entry", "StartupWMClass");
                    let stem = p.file_stem()?.to_str()?.to_lowercase();
                    let last = stem.rsplit('.').next().unwrap_or(&stem).to_string();
                    let hit = class.is_some_and(|c| c.eq_ignore_ascii_case(app))
                        || stem == lower
                        || last == lower;
                    hit.then_some(text)
                })
        })
    })?;
    ini_value(&text, "Desktop Entry", "Icon").filter(|i| !i.is_empty())
}

/// The file for icon `name` in `themes` (and what they inherit), then hicolor, then pixmaps.
fn theme_icon(name: &str, data: &[PathBuf], themes: &[String]) -> Option<PathBuf> {
    let mut chain: Vec<String> = Vec::new();
    for t in themes {
        inherits(t, data, &mut chain);
    }
    if !chain.iter().any(|t| t == "hicolor") {
        chain.push("hicolor".into());
    }
    for theme in &chain {
        for d in data {
            let root = d.join("icons").join(theme);
            if !root.is_dir() {
                continue;
            }
            // hicolor's layout (`scalable/apps`, `48x48/apps`) and Breeze's (`apps/48`).
            let mut dirs = vec![root.join("scalable/apps"), root.join("apps/scalable")];
            for s in SIZES {
                dirs.push(root.join(format!("{s}x{s}/apps")));
                dirs.push(root.join(format!("apps/{s}")));
            }
            for dir in dirs {
                for ext in ["svg", "svgz", "png"] {
                    let p = dir.join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    data.iter()
        .flat_map(|d| ["svg", "png"].map(|ext| d.join("pixmaps").join(format!("{name}.{ext}"))))
        .find(|p| p.is_file())
}

/// Adds `theme` and the themes it inherits (from its `index.theme`) to `chain`.
fn inherits(theme: &str, data: &[PathBuf], chain: &mut Vec<String>) {
    if chain.iter().any(|t| t == theme) || chain.len() > 8 {
        return;
    }
    chain.push(theme.to_string());
    let index = data.iter().find_map(|d| {
        std::fs::read_to_string(d.join("icons").join(theme).join("index.theme")).ok()
    });
    if let Some(parents) = index.and_then(|i| ini_value(&i, "Icon Theme", "Inherits")) {
        for p in parents.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            inherits(p, data, chain);
        }
    }
}

/// `key`'s value in `[section]` of an ini or desktop file.
fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
        } else if inside {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// A fixture XDG data dir for tests: a Konsole desktop file, its icon as scalable SVG and as
/// a 48 px PNG in hicolor, and an app known only by its window class. Made in code.
#[cfg(test)]
pub(crate) fn fixture(root: &Path) -> PathBuf {
    let data = root.join("share");
    let apps = data.join("applications");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        apps.join("org.kde.konsole.desktop"),
        "[Desktop Entry]\nName=Konsole\nIcon=utilities-terminal\nStartupWMClass=konsole\n[Desktop Action New]\nIcon=wrong\n",
    )
    .unwrap();
    std::fs::write(
        apps.join("org.example.Editor.desktop"),
        "[Desktop Entry]\nName=Editor\nIcon=example-editor\nStartupWMClass=ExampleEdit\n",
    )
    .unwrap();
    let scalable = data.join("icons/hicolor/scalable/apps");
    let png = data.join("icons/hicolor/48x48/apps");
    std::fs::create_dir_all(&scalable).unwrap();
    std::fs::create_dir_all(&png).unwrap();
    std::fs::write(scalable.join("utilities-terminal.svg"), FIXTURE_SVG).unwrap();
    std::fs::write(png.join("utilities-terminal.png"), b"\x89PNG not read").unwrap();
    std::fs::write(png.join("example-editor.png"), b"\x89PNG not read").unwrap();
    data
}

/// A terminal glyph on a gradient tile.
#[cfg(test)]
pub(crate) const FIXTURE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
<defs><linearGradient id="g" x2="1" y2="1"><stop offset="0" stop-color="#4cc9f0"/><stop offset="1" stop-color="#7b2ff7"/></linearGradient></defs>
<rect x="2" y="3" width="20" height="18" rx="4" fill="url(#g)"/><path d="M6 9l4 3-4 3M12 16h6" stroke="#fff" stroke-width="2" fill="none"/></svg>"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vo-app-icon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn konsole_finds_its_scalable_icon() {
        let root = tmp("konsole");
        let data = fixture(&root);
        let found = lookup("org.kde.konsole", std::slice::from_ref(&data), &[]);
        assert_eq!(
            found,
            Some(data.join("icons/hicolor/scalable/apps/utilities-terminal.svg"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_window_class_finds_the_desktop_file() {
        let root = tmp("class");
        let data = fixture(&root);
        assert_eq!(
            lookup("ExampleEdit", std::slice::from_ref(&data), &[]),
            Some(data.join("icons/hicolor/48x48/apps/example-editor.png"))
        );
        // The class names Konsole's desktop file too, ignoring case.
        assert!(lookup("Konsole", std::slice::from_ref(&data), &[]).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_app_has_no_icon() {
        let root = tmp("unknown");
        let data = fixture(&root);
        assert_eq!(lookup("org.example.Nothing", &[data], &[]), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_kde_theme_and_what_it_inherits_come_before_hicolor() {
        let root = tmp("theme");
        let data = fixture(&root);
        let dark = data.join("icons/breeze-dark");
        std::fs::create_dir_all(&dark).unwrap();
        std::fs::write(
            dark.join("index.theme"),
            "[Icon Theme]\nName=Breeze Dark\nInherits=breeze,hicolor\n",
        )
        .unwrap();
        let breeze = data.join("icons/breeze/apps/48");
        std::fs::create_dir_all(&breeze).unwrap();
        std::fs::write(breeze.join("utilities-terminal.svg"), FIXTURE_SVG).unwrap();
        assert_eq!(
            lookup(
                "org.kde.konsole",
                std::slice::from_ref(&data),
                &["breeze-dark".into()]
            ),
            Some(breeze.join("utilities-terminal.svg"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_full_path_icon_is_used_as_it_is() {
        let root = tmp("abs");
        let data = fixture(&root);
        let icon = root.join("own.svg");
        std::fs::write(&icon, FIXTURE_SVG).unwrap();
        std::fs::write(
            data.join("applications/own.desktop"),
            format!("[Desktop Entry]\nIcon={}\n", icon.display()),
        )
        .unwrap();
        assert_eq!(lookup("own", &[data], &[]), Some(icon));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ini_values_come_from_their_own_section() {
        let text = "[Desktop Entry]\nIcon=a\n[Desktop Action X]\nIcon=b\n";
        assert_eq!(
            ini_value(text, "Desktop Entry", "Icon").as_deref(),
            Some("a")
        );
        assert_eq!(ini_value(text, "Desktop Entry", "Name"), None);
    }
}
