//! `[tray]`: the StatusNotifierItem tray icon (Linux only).
//!
//! Off unless the user asks for it, so an upgrade never adds an icon to
//! someone's panel. Upstream voxtype turns its tray on by default; this
//! build does not.

use serde::{Deserialize, Serialize};

/// Tray icon configuration. The settings parse on every platform; the tray
/// itself only exists on Linux.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TrayConfig {
    /// Show a tray icon (KDE Plasma, or GNOME with the AppIndicator
    /// extension) with the daemon's state and a small menu.
    #[serde(default)]
    pub enabled: bool,

    /// How many recent dictations the tray menu lists. 0 hides the list.
    #[serde(default = "default_recent")]
    pub recent: u32,
}

fn default_recent() -> u32 {
    10
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            recent: default_recent(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrap {
        #[serde(default)]
        tray: TrayConfig,
    }

    #[test]
    fn tray_is_off_by_default() {
        assert!(!TrayConfig::default().enabled);
        let w: Wrap = toml::from_str("").unwrap();
        assert_eq!(w.tray, TrayConfig::default());
        assert!(!w.tray.enabled);
        assert_eq!(w.tray.recent, 10);
        let w: Wrap = toml::from_str("[tray]\n").unwrap();
        assert!(!w.tray.enabled);
    }

    #[test]
    fn tray_keys_parse() {
        let w: Wrap = toml::from_str("[tray]\nenabled = true\nrecent = 0\n").unwrap();
        assert!(w.tray.enabled);
        assert_eq!(w.tray.recent, 0);
    }
}
