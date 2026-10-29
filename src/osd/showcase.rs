//! The `showcase` OSD look: a deliberately rich popup that shows what the vo
//! GPU overlay framework can draw. It keeps the `swiss` popup's behaviour
//! (states, five transcript lines, waveform freeze, close rules) and changes
//! only the look.
//!
//! The idea is a glass instrument panel with the voice as light passing
//! through it: the level is a glowing ribbon, light leaks out of it as a
//! tilted ring of particles and drifting orbs, and the transcript sits on a calm
//! matte strip in the middle where nothing moves but the words.
//!
//! This module holds the palette, sizes and the geometry the view needs, so it
//! can be tested without a GPU. The vo frontend draws it.

use crate::osd::visual::Color;

/// Card body, near black with a blue cast.
pub const INK: Color = Color::rgb(0.043, 0.059, 0.102);
/// Top of the card's gradient.
pub const INK_HIGH: Color = Color::rgb(0.075, 0.098, 0.165);
/// The matte strip under the transcript.
pub const MATTE: Color = Color::rgb(0.031, 0.043, 0.075);
/// The one hot colour: rim light, flares, the final check.
pub const EMBER: Color = Color::rgb(1.0, 0.478, 0.102);
/// Text and type.
pub const ICE: Color = Color::rgb(0.812, 0.910, 1.0);
/// The live signal: ribbon, ring, progress.
pub const TEAL: Color = Color::rgb(0.180, 0.769, 0.714);
/// Off state, hairlines, the frozen ribbon.
pub const SLATE: Color = Color::rgb(0.353, 0.392, 0.471);

/// Transparent room around the card for glow, the ring and the orbs.
pub const MARGIN_PX: f32 = 64.0;
/// Corner radius of the card.
pub const RADIUS_PX: f32 = 14.0;
/// Height of the ribbon strip.
pub const RIBBON_PX: f32 = 56.0;
/// Seconds per lap of the light head around the card's rim.
pub const ORBIT_LAP_SECS: f64 = 6.0;
/// Ambient frame rate cap for the ring, the orbit and the orbs.
pub const AMBIENT_HZ: f64 = 30.0;
/// How long the card takes to open, during which ambient frames run.
pub const OPEN_SECS: f64 = 0.6;
/// How long the glow pulse after the final text lasts.
pub const FINAL_PULSE_SECS: f64 = 0.5;

/// Where the popup is in a dictation, from what the daemon says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The live model loads; the recording already runs.
    Loading,
    /// Recording, with speech in the last moments.
    Listening,
    /// Recording, but quiet.
    Paused,
    /// The recording stopped and the final text is being made.
    Finishing,
    /// The final text came and the daemon went idle.
    Done,
}

impl Phase {
    /// Whether the ambient loops (ring, orbit, orbs) run.
    pub fn ambient(self) -> bool {
        matches!(self, Phase::Loading | Phase::Listening | Phase::Paused)
    }
}

/// A level under this counts as quiet.
pub const QUIET_LEVEL: f32 = 0.04;
/// Quiet for this long counts as a pause.
pub const PAUSE_SECS: f64 = 0.6;

/// The phase from the status line flags, whether the final text came, and
/// how long the level has been quiet.
pub fn phase(loading: bool, finishing: bool, final_seen: bool, quiet_secs: f64) -> Phase {
    if final_seen && !finishing {
        Phase::Done
    } else if finishing {
        Phase::Finishing
    } else if loading {
        Phase::Loading
    } else if quiet_secs >= PAUSE_SECS {
        Phase::Paused
    } else {
        Phase::Listening
    }
}

/// Lit dots out of `n` for a progress fraction, rounded so a started job
/// lights at least one and only a finished one lights all.
pub fn lit_dots(fraction: f32, n: usize) -> usize {
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0 {
        return 0;
    }
    if f >= 1.0 {
        return n;
    }
    ((f * n as f32).round() as usize).clamp(1, n.saturating_sub(1))
}

/// Mix `a` toward `b` by `k` (0..1).
pub fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    Color::rgba(
        a.r + (b.r - a.r) * k,
        a.g + (b.g - a.g) * k,
        a.b + (b.b - a.b) * k,
        a.a + (b.a - a.a) * k,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_follow_the_daemon() {
        assert_eq!(phase(true, false, false, 0.0), Phase::Loading);
        assert_eq!(phase(false, false, false, 0.1), Phase::Listening);
        assert_eq!(phase(false, false, false, 1.0), Phase::Paused);
        assert_eq!(phase(false, true, false, 1.0), Phase::Finishing);
        assert_eq!(phase(false, false, true, 1.0), Phase::Done);
        assert!(Phase::Paused.ambient() && !Phase::Finishing.ambient());
    }

    #[test]
    fn dots_light_with_progress() {
        assert_eq!(lit_dots(0.0, 16), 0);
        assert_eq!(lit_dots(0.01, 16), 1);
        assert_eq!(lit_dots(0.5, 16), 8);
        assert_eq!(lit_dots(0.99, 16), 15);
        assert_eq!(lit_dots(1.0, 16), 16);
    }
}
