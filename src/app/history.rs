//! `voxtype history`: the last dictation, shown or copied to the clipboard.

use anyhow::bail;
use serde_json::{json, Value};
use voxtype::last_dictation::{self, LastDictation};
use voxtype::{config, output, HistoryAction};

/// What `voxtype history copy` says when there is nothing to copy.
pub(crate) const NOTHING_KEPT: &str = "No dictation is kept yet. Voxtype keeps the last \
dictation's text once one has been made since you logged in.";

pub(crate) async fn run_history_command(
    action: Option<HistoryAction>,
    json_output: bool,
) -> anyhow::Result<()> {
    let result = run(action, last_dictation::read()).await;
    match result {
        Ok(value) => {
            if json_output {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                print_plain(&value);
            }
            Ok(())
        }
        Err(e) if json_output => {
            println!("{}", json!({"error": e.to_string()}));
            std::process::exit(1);
        }
        Err(e) => Err(e),
    }
}

async fn run(action: Option<HistoryAction>, kept: Option<LastDictation>) -> anyhow::Result<Value> {
    match action {
        None => {
            let last = pick(kept, None)?;
            Ok(serde_json::to_value(last)?)
        }
        Some(HistoryAction::Copy { id }) => {
            let last = pick(kept, id.as_deref())?;
            let driver = copy(&last.text).await?;
            Ok(json!({"copied": "last", "driver": driver, "chars": last.text.chars().count()}))
        }
    }
}

/// The kept dictation `id` names: `last`, 1 or none. Others need history.
pub(crate) fn pick(kept: Option<LastDictation>, id: Option<&str>) -> anyhow::Result<LastDictation> {
    match id {
        None | Some("last") | Some("1") => {}
        Some(other) => bail!(
            "Only the last dictation is kept, so `{other}` cannot be found. Use \
             `voxtype history copy` or `voxtype history copy last`."
        ),
    }
    match kept {
        Some(last) => Ok(last),
        None => bail!("{NOTHING_KEPT}"),
    }
}

/// Put `text` on the clipboard with the first clipboard driver that works.
async fn copy(text: &str) -> anyhow::Result<&'static str> {
    let clipboard = config::OutputConfig {
        mode: config::OutputMode::Clipboard,
        append_text: None,
        ..Default::default()
    };
    let mut errors = Vec::new();
    for driver in output::create_output_chain(&clipboard) {
        if driver.is_available().await {
            match driver.output(text).await {
                Ok(()) => return Ok(driver.name()),
                Err(e) => errors.push(e.to_string()),
            }
        }
    }
    bail!(
        "No clipboard driver succeeded. Install wl-copy (Wayland), xclip (X11), or pbcopy \
         (macOS). {}",
        errors.join("; ")
    )
}

fn print_plain(value: &Value) {
    if let Some(driver) = value.get("copied").and(value.get("driver")) {
        println!(
            "Copied the last dictation ({} characters) with {}.",
            value["chars"],
            driver.as_str().unwrap_or("?")
        );
        return;
    }
    let delivered = if value["delivered"].as_bool() == Some(true) {
        format!("delivered with {}", value["method"].as_str().unwrap_or("?"))
    } else {
        format!(
            "not delivered: {}",
            value["error"].as_str().unwrap_or("unknown error")
        )
    };
    println!("{}  ({delivered})", value["time"].as_str().unwrap_or(""));
    println!("{}", value["text"].as_str().unwrap_or(""));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept() -> Option<LastDictation> {
        Some(LastDictation::now("A made-up dictation.", Ok("paste")))
    }

    #[test]
    fn copy_takes_the_last_dictation_by_default() {
        for id in [None, Some("last"), Some("1")] {
            assert_eq!(pick(kept(), id).unwrap().text, "A made-up dictation.");
        }
    }

    #[test]
    fn an_older_id_needs_history() {
        let e = pick(kept(), Some("2")).unwrap_err().to_string();
        assert!(e.contains("Only the last dictation is kept"), "{e}");
    }

    #[test]
    fn nothing_kept_says_so() {
        let e = pick(None, None).unwrap_err().to_string();
        assert_eq!(e, NOTHING_KEPT);
    }
}
