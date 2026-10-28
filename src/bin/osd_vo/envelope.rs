//! `voxtype-osd-vo --envelope`: runs the scripted dictation of `--offline`
//! through the reading-line envelope check (MO-08, `vo::envelope`) and, when
//! the GPU has timestamps, measures the look's GPU time per frame.
//!
//! Prints both reports to stdout. Exits with an error when the envelope is
//! breached or the 95th percentile frame is over budget, so a script can
//! loop over looks. `VO_REDUCED_MOTION=1` checks the reduced-motion form.

use vo::envelope::{check_scenario, measure_budget, Envelope};
use vo::{FontRegistry, Options};

use super::{default_motion, scenario, Look, Popup};

/// Whole popup GPU time per frame (motion plan, section 2, rule 8).
const FRAME_MS: f64 = 1.0;
/// No single section over this.
const SECTION_MS: f64 = 2.0;

pub(crate) fn run(look: Look, fonts: FontRegistry, fps: f64, cancel: bool) -> anyhow::Result<()> {
    let scenario = scenario(&look, cancel);
    let options = || Options {
        fonts: fonts.clone(),
        default_motion: default_motion(&look),
        renderer_setup: Some(std::sync::Arc::new(vo_3d::install)),
        ..Options::default()
    };
    let report = check_scenario(
        Popup::new(look.clone(), &fonts),
        &scenario,
        fps,
        options(),
        &Envelope::default(),
    );
    println!("envelope:\n{report}");
    let budget = measure_budget(Popup::new(look.clone(), &fonts), &scenario, fps, options())
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
