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

use vo::envelope::{check_scenario, measure_budget, Envelope};
use vo::{FontRegistry, Options};

use super::{default_motion, scenario, Look, Popup};

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
) -> anyhow::Result<()> {
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
    let options = || Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        renderer_setup: Some(std::sync::Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let envelope = Envelope {
        transcript: 0..TRANSCRIPT_IDS,
        ..Envelope::default()
    };
    // Slower, a word that holds still for less than two frames before it
    // moves on never shows its rest there, and reads as a breach it is not
    // (y50p: jarvis at 10 fps).
    let slowest = envelope.slowest_fps();
    if fps < slowest {
        anyhow::bail!(
            "--envelope needs --fps {slowest} or more: at {fps} fps a word that \
             holds still for less than two frames is not seen at rest"
        );
    }
    let report = check_scenario(popup(), &scenario, fps, options(), &envelope);
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
    let budget = measure_budget(popup(), &scenario, fps, options())
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
