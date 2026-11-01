//! `voxtype-osd-vo --envelope`: runs the scripted dictation of `--offline`
//! through the reading-line envelope check (MO-08, `vo::envelope`) and, when
//! the GPU has timestamps, measures the look's GPU time per frame.
//!
//! Prints both reports to stdout. Exits with an error when the envelope is
//! breached or the 95th percentile frame is over budget, so a script can
//! loop over looks. `VO_REDUCED_MOTION=1` checks the reduced-motion form.
//! With `--moment NAME` it checks that moment instead. `VO_ENVELOPE_DETAIL=1`
//! also prints every breach, one line each (frame, time, word id, kind,
//! value); it is off by default.
//!
//! While a correction key is held the live words show at
//! [`correction_note::DIMMED`] of their ink by design (COR-3), so in those
//! spans the ink floor is taken against the dimmed ink ([`report`]).

use vo::envelope::{measure_budget, sample_scenario, Envelope, Report, Tracker};
use vo::scene::Motion;
use vo::{FontRegistry, Input, Options};

use super::{correction_note, default_motion, scenario, Look, Msg, Popup};

/// Whole popup GPU time per frame (motion plan, section 2, rule 8).
const FRAME_MS: f64 = 1.0;
/// No single section over this.
const SECTION_MS: f64 = 2.0;
/// Transcript word ids count up from 0. The live words of a status row carry ids hashed
/// from their label, almost always far above this, and must not set the reading line.
const TRANSCRIPT_IDS: u32 = 1 << 20;

pub(crate) fn run(
    look: Look,
    fonts: FontRegistry,
    fps: f64,
    cancel: bool,
    moment: Option<&str>,
    replay: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    if let Some(path) = replay {
        let events = super::replay::load(path)?;
        let lookahead = super::replay::Lookahead::scan(&events);
        let scenario = super::replay::scenario(&look, events);
        let popup = || Popup::replayed(look.clone(), &fonts, lookahead.clone());
        return check(&look, &fonts, fps, &scenario, popup);
    }
    let moment = moment
        .map(|name| {
            super::moments::Moment::parse(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "no moment {name:?}; the moments are {}",
                    super::moments::Moment::names()
                )
            })
        })
        .transpose()?;
    let scenario = match moment {
        Some(m) => super::moments::scenario(&look, m),
        None => scenario(&look, cancel),
    };
    // The edit moment's popup has an edit link; with it the envelope's
    // reading line is the caret line (M4.9).
    let popup = || {
        let p = Popup::new(look.clone(), &fonts);
        match moment {
            Some(m) => super::moments::popup_for(m, p),
            None => p,
        }
    };
    check(&look, &fonts, fps, &scenario, popup)
}

/// The envelope and the budget of `scenario` played by the popups `popup` makes.
fn check(
    look: &Look,
    fonts: &FontRegistry,
    fps: f64,
    scenario: &vo::Scenario<super::Msg>,
    popup: impl Fn() -> Popup,
) -> anyhow::Result<()> {
    let options = || Options {
        fonts: fonts.clone(),
        default_motion: default_motion(look),
        renderer_setup: Some(std::sync::Arc::new(vo_3d::install)),
        ..crate::kit::options()
    };
    let envelope = Envelope {
        transcript: 0..TRANSCRIPT_IDS,
        ..Envelope::default()
    };
    // Slower, a word that holds still for about 100 ms before it moves on
    // never shows its rest there, and reads as a breach it is not (y50p:
    // jarvis at 10 fps; m3qn: jarvis's depth stack at 15 to 24 fps).
    let slowest = envelope.slowest_fps();
    if fps < slowest {
        anyhow::bail!(
            "--envelope needs --fps {slowest} or more: at {fps} fps a word that \
             holds still for about 100 ms is not seen at rest"
        );
    }
    let report = report(popup(), scenario, fps, options(), &envelope);
    println!("envelope:\n{report}");
    // Development: every breach, one line each, to find where a look goes wrong.
    if std::env::var_os("VO_ENVELOPE_DETAIL").is_some() {
        for v in &report.violations {
            println!(
                "breach frame {} t {:.3} word {} {} {:.3}",
                v.frame,
                v.t,
                v.word,
                v.breach.kind(),
                v.breach.value()
            );
        }
    }
    let budget = measure_budget(popup(), scenario, fps, options())
        .map_err(|e| anyhow::anyhow!("offline render: {e}"))?;
    let mut over = None;
    match &budget {
        Some(b) => {
            println!("budget:\n{b}");
            over = b.within(FRAME_MS, SECTION_MS).err();
            if let Some(why) = &over {
                println!("budget: over ({why})");
            }
        }
        None => println!("budget: this GPU has no timestamps"),
    }
    if !report.is_ok() || over.is_some() {
        anyhow::bail!("envelope or budget breached");
    }
    Ok(())
}

/// The spans of `scenario` in which a correction key is held, from the
/// files that open a hold to the files that close it (COR-3).
fn holds(scenario: &vo::Scenario<Msg>) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut open = None;
    for (t, input) in &scenario.inputs {
        let Input::Msg(Msg::Files(f)) = input else {
            continue;
        };
        let view = f.live.as_ref().and_then(|l| l.correction.as_ref());
        let held = view.is_some_and(|v| v.hearing.is_some());
        match (open, held) {
            (None, true) => open = Some(*t),
            (Some(from), false) => {
                out.push((from, *t));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(from) = open {
        out.push((from, f64::INFINITY));
    }
    out
}

/// How long `motion` takes to bring the dimmed words back up to `ink` of
/// their full ink once the key is let go: none for a snap, the spring's
/// time from [`correction_note::DIMMED`] to `ink` otherwise.
fn fade_back(motion: &Motion, ink: f32) -> f64 {
    let Motion::Spring(spring) = motion else {
        return 0.0;
    };
    let from = f64::from(correction_note::DIMMED) - 1.0;
    let mut t = 0.0;
    while t < 2.0 && 1.0 + spring.eval(from, 0.0, t).0 < f64::from(ink) {
        t += 0.001;
    }
    t
}

/// `envelope` for each span of `scenario`: as given outside the holds, and
/// inside them, and for `tail` seconds after each while the words come
/// back up, with the ink floor taken against the dimmed ink. A glyph
/// dimmed further than the correction dims it still breaches.
fn spans(envelope: &Envelope, holds: &[(f64, f64)], tail: f64) -> Vec<Envelope> {
    // Spans are `from..=until`; this keeps a frame on a boundary in one.
    const BEFORE: f64 = 1e-9;
    let mut out = Vec::new();
    let mut from = envelope.from;
    for &(start, end) in holds {
        let end = end + tail;
        out.push(Envelope {
            from,
            until: (start - BEFORE).min(envelope.until),
            ..envelope.clone()
        });
        out.push(Envelope {
            from: start.max(envelope.from),
            until: (end - BEFORE).min(envelope.until),
            ink: envelope.ink * correction_note::DIMMED,
            ..envelope.clone()
        });
        from = end.max(envelope.from);
    }
    out.push(Envelope {
        from,
        ..envelope.clone()
    });
    out
}

/// The reading-line envelope of `scenario` played by `popup`, with the
/// correction holds checked against their dimmed ink ([`spans`]).
pub(crate) fn report(
    popup: Popup,
    scenario: &vo::Scenario<Msg>,
    fps: f64,
    options: Options<Msg>,
    envelope: &Envelope,
) -> Report {
    let tail = fade_back(&options.default_motion, envelope.ink);
    let mut tracker = Tracker::new();
    sample_scenario(popup, scenario, fps, options, &mut |_, t, list| {
        tracker.push(t, list)
    });
    let mut out: Option<Report> = None;
    for env in spans(envelope, &holds(scenario), tail) {
        let r = tracker.check(&env);
        match &mut out {
            None => out = Some(r),
            Some(o) => {
                // `frames` and `leaving` count every frame, whatever the span.
                o.frames_checked += r.frames_checked;
                o.glyphs_checked += r.glyphs_checked;
                o.violations.extend(r.violations);
            }
        }
    }
    let mut out = out.unwrap_or_default();
    out.violations.sort_by_key(|v| v.frame);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_fonts, OsdConfig, OsdTheme, PlasmaTheme};
    use vo::envelope::{check_scenario, Breach};

    use super::super::moments::{self, Moment};

    /// Both correction moments in Swiss (snap) and Type (soft spring):
    /// checked as one span, only the held words breach the ink floor, from
    /// the dimmed ink up, inside a hold or its fade back; with those spans
    /// checked against the dimmed ink, they pass.
    #[test]
    fn a_held_correction_dims_the_words_by_design() {
        for theme in [OsdTheme::Swiss, OsdTheme::Type] {
            held_correction_in(theme);
        }
    }

    fn held_correction_in(look_theme: OsdTheme) {
        let theme = PlasmaTheme::default();
        let fonts = load_fonts(&theme).unwrap();
        let mut look = Look::new(&OsdConfig::default(), &theme, String::new(), &fonts);
        look.theme = look_theme;
        let tail = fade_back(&default_motion(&look), Envelope::default().ink);
        let options = || Options {
            fonts: fonts.clone(),
            default_motion: default_motion(&look),
            ..crate::kit::options()
        };
        let envelope = Envelope {
            transcript: 0..TRANSCRIPT_IDS,
            ..Envelope::default()
        };
        for m in [Moment::Correction, Moment::Spell] {
            let s = moments::scenario(&look, m);
            let holds = holds(&s);
            assert!(!holds.is_empty(), "{m:?} holds the key");
            let popup = || Popup::new(look.clone(), &fonts);
            let plain = check_scenario(popup(), &s, 30.0, options(), &envelope);
            assert!(!plain.is_ok(), "{look_theme:?} {m:?}: the held words dim");
            for v in &plain.violations {
                let held = holds.iter().any(|&(a, b)| v.t >= a && v.t < b + tail);
                assert!(
                    matches!(v.breach, Breach::Ink(i) if i > correction_note::DIMMED - 0.01)
                        && held,
                    "{look_theme:?} {m:?}: only the dimmed ink breaches, and only while held: {v:?}"
                );
            }
            let r = report(popup(), &s, 30.0, options(), &envelope);
            assert!(r.is_ok(), "{m:?}: {r}");
            assert_eq!(
                r.frames_checked, plain.frames_checked,
                "{m:?}: every frame in one span"
            );
        }
    }

    /// The default soft spring takes about 63 ms from 35 % back to 70 %
    /// ink: two frames at 30 fps. A snap takes none.
    #[test]
    fn the_fade_back_is_the_springs_time_to_the_floor() {
        let soft = fade_back(&Motion::Spring(vo_core::spring::Spring::SOFT), 0.7);
        assert!((0.055..0.075).contains(&soft), "{soft}");
        assert_eq!(fade_back(&Motion::Snap, 0.7), 0.0);
    }

    #[test]
    fn the_spans_cover_the_check_once() {
        let env = Envelope::default();
        let s = spans(&env, &[(1.0, 2.0), (3.0, f64::INFINITY)], 0.1);
        let inks: Vec<f32> = s.iter().map(|e| e.ink).collect();
        let dim = env.ink * correction_note::DIMMED;
        assert_eq!(inks, [env.ink, dim, env.ink, dim, env.ink]);
        let inside = |t: f64| s.iter().filter(|e| t >= e.from && t <= e.until).count();
        for t in [0.0, 1.0, 1.5, 2.0, 2.1, 2.5, 3.0, 9.0] {
            assert_eq!(inside(t), 1, "t {t}");
        }
        let dimmed = |t: f64| {
            s.iter()
                .any(|e| e.ink == dim && t >= e.from && t <= e.until)
        };
        assert!(
            dimmed(2.05) && !dimmed(2.1),
            "the tail ends 0.1 s after the release"
        );
    }
}
