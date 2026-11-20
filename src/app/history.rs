//! `voxtype history`: list, show, copy and manage recorded dictations.
//!
//! Ported from upstream voxtype v1.3.1 (the list, show, copy, delete,
//! clear, pause, resume, enable, disable, retention and status actions).
//! With history off or paused, `last` falls back to the last dictation the
//! daemon keeps in the runtime folder ([`last_dictation`]).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{bail, Context};
use chrono::NaiveDate;
use serde_json::{json, Value};
use voxtype::history::{HistoryEntry, HistoryStore};
use voxtype::last_dictation::{self, LastDictation};
use voxtype::{config, config_set, output, HistoryAction};

/// What `voxtype history copy` says when there is nothing to copy.
pub(crate) const NOTHING_KEPT: &str = "No dictation is kept yet. Voxtype keeps the last \
dictation's text once one has been made since you logged in.";

pub(crate) async fn run_history_command(
    config: &config::Config,
    config_path: Option<PathBuf>,
    action: Option<HistoryAction>,
    json_output: bool,
) -> anyhow::Result<()> {
    let store = HistoryStore::at(HistoryStore::default_dir());
    let result = run(config, config_path, &store, action).await;
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

async fn run(
    config: &config::Config,
    config_path: Option<PathBuf>,
    store: &HistoryStore,
    action: Option<HistoryAction>,
) -> anyhow::Result<Value> {
    let history = &config.history;
    let action = match action {
        Some(action) => action,
        // History off: the last dictation, as before history existed.
        None if !history.enabled => {
            return match pick(history, store, last_dictation::read(), None)? {
                Picked::Entry(entry) => Ok(serde_json::to_value(entry)?),
                Picked::Last(last) => Ok(serde_json::to_value(last)?),
            };
        }
        None => HistoryAction::List {
            limit: 20,
            failed: false,
        },
    };
    match action {
        HistoryAction::List { limit, failed } => {
            let entries = if failed {
                let mut entries = store.all()?;
                entries.retain(|e| !e.delivered);
                entries.sort_by_key(|e| std::cmp::Reverse(e.ts_start));
                entries.truncate(limit);
                entries
            } else {
                store.recent(limit)?
            };
            let mut result = state(history, store)?;
            result["entries"] = serde_json::to_value(entries)?;
            Ok(result)
        }
        HistoryAction::Status => state(history, store),
        HistoryAction::Show { id } => {
            match pick(history, store, last_dictation::read(), Some(&id))? {
                Picked::Entry(entry) => Ok(serde_json::to_value(entry)?),
                Picked::Last(last) => Ok(serde_json::to_value(last)?),
            }
        }
        HistoryAction::Copy { id } => {
            let picked = pick(history, store, last_dictation::read(), id.as_deref())?;
            let (copied, text) = match &picked {
                Picked::Entry(entry) => (entry.id.as_str(), entry.best_text()),
                Picked::Last(last) => ("last", last.text.as_str()),
            };
            let driver = copy(text).await?;
            Ok(json!({"copied": copied, "driver": driver, "chars": text.chars().count()}))
        }
        HistoryAction::Delete { id } => {
            let entry = find(store, &id)?;
            store
                .delete(&entry.id)?
                .context("Dictation was already deleted")?;
            Ok(json!({"deleted": entry.id}))
        }
        HistoryAction::Clear { before, yes } => {
            let before = date(before)?;
            if !yes {
                if !std::io::stdin().is_terminal() {
                    bail!("Use --yes to confirm clearing history when not on a terminal");
                }
                eprint!(
                    "Delete {} history? Type yes: ",
                    before
                        .map(|d| format!("days before {d}"))
                        .unwrap_or_else(|| "all".into())
                );
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if answer.trim() != "yes" {
                    bail!("History was not cleared");
                }
            }
            let removed = store.clear(before)?;
            let mut result = json!({"days": removed.len()});
            if before.is_none() {
                result["last_dictation_removed"] = json!(remove_last_dictation()?);
            }
            Ok(result)
        }
        HistoryAction::Pause
        | HistoryAction::Resume
        | HistoryAction::Enable
        | HistoryAction::Disable => {
            let (key, value) = match action {
                HistoryAction::Pause => ("history.paused", "true"),
                HistoryAction::Resume => ("history.paused", "false"),
                HistoryAction::Enable => ("history.enabled", "true"),
                HistoryAction::Disable => ("history.enabled", "false"),
                _ => unreachable!(),
            };
            let path = super::config_set::resolve_config_path_for_write(config_path)?;
            config_set::set_key(path.clone(), key, value)?;
            let saved = config::load_config(Some(&path))?;
            state(&saved.history, store)
        }
        HistoryAction::Retention { days } => {
            let path = super::config_set::resolve_config_path_for_write(config_path)?;
            config_set::set_key(path.clone(), "history.retention_days", &days.to_string())?;
            let saved = config::load_config(Some(&path))?;
            state(&saved.history, store)
        }
    }
}

/// A dictation `pick` found: a history entry, or the last dictation kept in
/// the runtime folder.
#[derive(Debug)]
pub(crate) enum Picked {
    Entry(HistoryEntry),
    Last(LastDictation),
}

/// Whether `id` names the newest dictation.
fn is_last(id: Option<&str>) -> bool {
    matches!(id, None | Some("last") | Some("1"))
}

/// The dictation `id` names, for show and copy. The newest (no ID, `last`
/// or 1) comes from history while it records; while it is off or paused it
/// is the last dictation kept in the runtime folder, which is newer than
/// anything in history. Older ones come from history.
pub(crate) fn pick(
    history: &config::HistoryConfig,
    store: &HistoryStore,
    kept: Option<LastDictation>,
    id: Option<&str>,
) -> anyhow::Result<Picked> {
    if is_last(id) {
        if history.recording() {
            if let Some(entry) = store.recent(1)?.into_iter().next() {
                return Ok(Picked::Entry(entry));
            }
        }
        if let Some(last) = kept {
            return Ok(Picked::Last(last));
        }
        if let Some(entry) = store.recent(1)?.into_iter().next() {
            return Ok(Picked::Entry(entry));
        }
        bail!("{NOTHING_KEPT}");
    }
    let id = id.unwrap_or("last");
    match find(store, id) {
        Ok(entry) => Ok(Picked::Entry(entry)),
        Err(_) if !history.enabled => bail!(
            "Only the last dictation is kept while history is off, so `{id}` cannot be \
             found. Use `voxtype history copy`, or turn history on with `voxtype history \
             enable`."
        ),
        Err(e) => Err(e),
    }
}

/// The history entry `id` names: an entry ID, `last`, or a position in the
/// list (1 is the newest).
pub(crate) fn find(store: &HistoryStore, id: &str) -> anyhow::Result<HistoryEntry> {
    let entry = match position(id) {
        Some(n) => store.recent(n)?.into_iter().nth(n - 1),
        None => store.find(id)?,
    };
    entry.with_context(|| format!("No dictation found for '{id}'"))
}

/// `last` is 1; a short number is a position. Entry IDs start with an
/// eight-digit date, so they never read as one.
fn position(id: &str) -> Option<usize> {
    if id == "last" {
        return Some(1);
    }
    if id.len() > 6 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok().filter(|n| *n > 0)
}

fn date(value: Option<String>) -> anyhow::Result<Option<NaiveDate>> {
    value
        .map(|s| {
            let day = NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .context("Use a date in YYYY-MM-DD format")?;
            if day.to_string() != s {
                bail!("Use a date in YYYY-MM-DD format");
            }
            Ok(day)
        })
        .transpose()
}

fn state(history: &config::HistoryConfig, store: &HistoryStore) -> anyhow::Result<Value> {
    Ok(json!({
        "enabled": history.enabled,
        "paused": history.paused,
        "recording": history.recording(),
        "retention_days": history.retention_days,
        "max_entries": history.max_entries,
        "count": store.all()?.len(),
        "dir": store.dir(),
    }))
}

/// Remove the last dictation kept in the runtime folder. True when there
/// was one.
fn remove_last_dictation() -> anyhow::Result<bool> {
    match std::fs::remove_file(last_dictation::path()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context("Could not remove the last kept dictation"),
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

/// One line of a dictation's text, for a list.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn print_plain(value: &Value) {
    if let Some(driver) = value.get("copied").and(value.get("driver")) {
        let what = match value["copied"].as_str() {
            Some("last") | None => "the last dictation".to_string(),
            Some(id) => id.to_string(),
        };
        println!(
            "Copied {what} ({} characters) with {}.",
            value["chars"],
            driver.as_str().unwrap_or("?")
        );
        return;
    }
    if let Some(entries) = value.get("entries").and_then(Value::as_array) {
        println!(
            "History: {}{}",
            if value["enabled"] == true {
                "on"
            } else {
                "off"
            },
            if value["paused"] == true {
                " (paused)"
            } else {
                ""
            }
        );
        for (n, entry) in entries.iter().enumerate() {
            let text = entry
                .get("corrected_text")
                .or(entry.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            println!(
                "{:>3}  {}  {}{}  {}",
                n + 1,
                entry["id"].as_str().unwrap_or(""),
                entry["outcome"].as_str().unwrap_or(""),
                if entry.get("corrected_text").is_some() {
                    " [corrected]"
                } else {
                    ""
                },
                one_line(text)
            );
        }
        if entries.is_empty() {
            println!("No dictations stored.");
        }
        return;
    }
    // The last dictation kept in the runtime folder.
    if value.get("schema").is_none() && value.get("delivered").is_some() {
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
        return;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use voxtype::history::Outcome;

    fn kept() -> Option<LastDictation> {
        Some(LastDictation::now("A made-up dictation.", Ok("paste")))
    }

    fn on() -> config::HistoryConfig {
        config::HistoryConfig {
            enabled: true,
            ..Default::default()
        }
    }

    fn off() -> config::HistoryConfig {
        config::HistoryConfig::default()
    }

    fn entry_at(h: u32, text: &str) -> HistoryEntry {
        let mut e = HistoryEntry::new(Local.with_ymd_and_hms(2026, 11, 14, h, 0, 0).unwrap());
        e.raw_text = text.to_string();
        e.text = text.to_string();
        e.set_outcome(Outcome::Delivered, None);
        e
    }

    /// A store with an older and a newer entry, written oldest first as the
    /// daemon writes them.
    fn store_with_two() -> (tempfile::TempDir, HistoryStore, HistoryEntry, HistoryEntry) {
        let t = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(t.path().join("history")).unwrap();
        let older = entry_at(9, "The older made-up line.");
        let newer = entry_at(10, "The newer made-up line.");
        store.append(&older).unwrap();
        store.append(&newer).unwrap();
        (t, store, older, newer)
    }

    fn empty_store() -> (tempfile::TempDir, HistoryStore) {
        let t = tempfile::tempdir().unwrap();
        let store = HistoryStore::at(t.path().join("history"));
        (t, store)
    }

    fn text_of(p: Picked) -> String {
        match p {
            Picked::Entry(e) => e.best_text().to_string(),
            Picked::Last(l) => l.text,
        }
    }

    #[test]
    fn with_history_on_copy_takes_the_newest_entry() {
        let (_t, store, _older, newer) = store_with_two();
        for id in [None, Some("last"), Some("1")] {
            match pick(&on(), &store, kept(), id).unwrap() {
                Picked::Entry(e) => assert_eq!(e.id, newer.id, "{id:?}"),
                other => panic!("{id:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn positions_and_ids_name_older_entries() {
        let (_t, store, older, newer) = store_with_two();
        assert_eq!(
            text_of(pick(&on(), &store, kept(), Some("2")).unwrap()),
            older.text
        );
        assert_eq!(
            text_of(pick(&on(), &store, None, Some(&newer.id)).unwrap()),
            newer.text
        );
        assert!(pick(&on(), &store, kept(), Some("3")).is_err());
        assert_eq!(find(&store, "last").unwrap().id, newer.id);
    }

    #[test]
    fn with_history_off_copy_takes_the_kept_dictation() {
        let (_t, store, _older, _newer) = store_with_two();
        for id in [None, Some("last"), Some("1")] {
            assert_eq!(
                text_of(pick(&off(), &store, kept(), id).unwrap()),
                "A made-up dictation."
            );
        }
        let (_t, empty) = empty_store();
        assert_eq!(
            text_of(pick(&off(), &empty, kept(), None).unwrap()),
            "A made-up dictation."
        );
    }

    #[test]
    fn paused_history_copies_the_kept_dictation_which_is_newer() {
        let (_t, store, _older, _newer) = store_with_two();
        let paused = config::HistoryConfig {
            paused: true,
            ..on()
        };
        assert_eq!(
            text_of(pick(&paused, &store, kept(), None).unwrap()),
            "A made-up dictation."
        );
    }

    #[test]
    fn history_on_but_empty_falls_back_to_the_kept_dictation() {
        let (_t, empty) = empty_store();
        assert_eq!(
            text_of(pick(&on(), &empty, kept(), Some("last")).unwrap()),
            "A made-up dictation."
        );
    }

    #[test]
    fn an_older_id_needs_history() {
        let (_t, empty) = empty_store();
        let e = pick(&off(), &empty, kept(), Some("2"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("Only the last dictation is kept"), "{e}");
    }

    #[test]
    fn nothing_kept_says_so() {
        let (_t, empty) = empty_store();
        let e = pick(&off(), &empty, None, None).unwrap_err().to_string();
        assert_eq!(e, NOTHING_KEPT);
    }

    #[test]
    fn positions_never_swallow_entry_ids() {
        assert_eq!(position("last"), Some(1));
        assert_eq!(position("1"), Some(1));
        assert_eq!(position("12"), Some(12));
        assert_eq!(position("0"), None);
        assert_eq!(position("20261114-090000-abcdef"), None);
        assert_eq!(position("20261114"), None);
    }

    #[tokio::test]
    async fn list_and_delete_work_on_the_store() {
        let (_t, store, older, newer) = store_with_two();
        let config = config::Config {
            history: on(),
            ..Default::default()
        };
        let listed = run(&config, None, &store, None).await.unwrap();
        let ids: Vec<_> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec![newer.id.clone(), older.id.clone()]);
        assert_eq!(listed["count"], 2);

        let deleted = run(
            &config,
            None,
            &store,
            Some(HistoryAction::Delete { id: "last".into() }),
        )
        .await
        .unwrap();
        assert_eq!(deleted["deleted"], json!(newer.id));
        assert_eq!(store.all().unwrap(), vec![older]);
    }

    #[tokio::test]
    async fn failed_lists_only_undelivered_dictations() {
        let (_t, store, _older, _newer) = store_with_two();
        let mut failed = entry_at(8, "A made-up line that never landed.");
        failed.set_outcome(Outcome::OutputFailed, Some("no driver".into()));
        store.append(&failed).unwrap();
        let config = config::Config {
            history: on(),
            ..Default::default()
        };
        let listed = run(
            &config,
            None,
            &store,
            Some(HistoryAction::List {
                limit: 20,
                failed: true,
            }),
        )
        .await
        .unwrap();
        let entries = listed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], json!(failed.id));
    }

    #[tokio::test]
    async fn pause_and_retention_write_the_given_config() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(&path, "[history]\nenabled = true\n").unwrap();
        let (_s, store) = empty_store();
        let config = config::Config::default();
        let paused = run(
            &config,
            Some(path.clone()),
            &store,
            Some(HistoryAction::Pause),
        )
        .await
        .unwrap();
        assert_eq!(paused["paused"], true);
        assert_eq!(paused["recording"], false);
        let kept = run(
            &config,
            Some(path.clone()),
            &store,
            Some(HistoryAction::Retention { days: 7 }),
        )
        .await
        .unwrap();
        assert_eq!(kept["retention_days"], 7);
        let saved = config::load_config(Some(&path)).unwrap();
        assert!(saved.history.paused);
        assert_eq!(saved.history.retention_days, 7);
    }
}
