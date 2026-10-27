//! Shared logic for the on-screen visualizer binaries.
//!
//! Both `voxtype-osd-native` (SCTK + wgpu + egui-wgpu) and `voxtype-osd-gtk4`
//! (GTK4 + gtk4-layer-shell) consume the same daemon IPC, run the same
//! peak-hold + waveform envelope math, parse the same Omarchy theme, and
//! honor the same `[osd]` configuration. That logic lives here so the two
//! frontends only differ in their rendering surface.
//!
//! ## Module layout
//!
//! - [`ipc`] — Unix-socket connection, frame decode, ring buffer, reconnect.
//! - [`live`]: the live preview as JSON (`preview.json`), with word timing.
//! - [`target`]: the app that has keyboard focus, for the paste toast.
//! - [`visual`] — peak-hold decay, waveform envelope helpers, palette types.
//! - [`config`] — `[osd]` config block (`OsdConfig`).
//! - [`theme`] — Omarchy theme parsing + change watcher.
//! - [`plasma`] — KDE Plasma colors and fonts for the `swiss` gtk4 theme.
//! - [`swiss`] — the `swiss` look shared by the gtk4 and vo frontends.
//! - [`showcase`] — palette and geometry of the vo frontend's `showcase` look.
//! - [`watch`]: wakes the popups when the daemon changes a runtime file.
//! - [`gtk_env`] — environment workarounds for GTK4 bugs (GTK frontend only).

pub mod config;
pub mod gtk_env;
pub mod ipc;
pub mod live;
pub mod outcome;
pub mod plasma;
pub mod recipe;
pub mod showcase;
pub mod style;
pub mod supervisor;
pub mod swiss;
pub mod target;
pub mod theme;
pub mod visual;
#[cfg(target_os = "linux")]
pub mod watch;
