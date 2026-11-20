//! `voxtype history` subcommands. Names and arguments follow upstream
//! voxtype v1.3.1; its search, paste, correct, rule, reindex and view
//! actions are not ported.

use clap::Subcommand;

pub const HISTORY_AFTER_HELP: &str = "\
With history on ([history] enabled = true, or `voxtype history enable`),
every dictation, delivered or not, is one JSON line in
~/.local/share/voxtype/history/YYYY-MM-DD.jsonl, readable by you alone.
With history off, voxtype keeps only the last dictation, in the runtime
folder ($XDG_RUNTIME_DIR/voxtype/last-dictation.json), until logout.
Every command takes --json.

An ID is what `voxtype history` lists (20261114-093012-a1b2c3), `last`, or
a position in that list: 1 is the newest.

EXAMPLES:
  voxtype history                      Recent dictations, newest first
                                       (the last one when history is off)
  voxtype history -n 50 --failed       The last 50 whose delivery failed
  voxtype history copy                 Copy the last dictation to the clipboard
  voxtype history copy 3               Copy the third newest
  voxtype history show last            One dictation in full, with its edits
  voxtype history pause                Stop recording before something private
  voxtype history resume
  voxtype history clear --before 2026-11-01

KEY BINDINGS:
  KDE Plasma:  System Settings > Shortcuts > Add Command: voxtype history copy
  Hyprland:    bind = SUPER SHIFT, V, exec, voxtype history copy
  Sway:        bindsym $mod+Shift+v exec voxtype history copy";

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum HistoryAction {
    /// List recent dictations, newest first (the default with history on)
    List {
        /// How many to show
        #[arg(short = 'n', long, value_name = "N", default_value_t = 20)]
        limit: usize,

        /// Only dictations whose delivery failed
        #[arg(long)]
        failed: bool,
    },

    /// Show one dictation in full, with its automatic edits
    Show {
        /// Entry ID, position (1 is the newest), or `last`
        id: String,
    },

    /// Copy a dictation's text to the clipboard (the most recent when no ID)
    ///
    /// Copies the correction if there is one, otherwise what was output,
    /// otherwise what the engine heard, so a failed dictation can still be
    /// recovered. With history off or paused, `last` is the last dictation
    /// voxtype kept in the runtime folder.
    Copy {
        /// Entry ID, position (1 is the newest), or `last`
        id: Option<String>,
    },

    /// Delete one dictation from history
    Delete {
        /// Entry ID, position (1 is the newest), or `last`
        id: String,
    },

    /// Delete whole days of history
    ///
    /// Without --before, also removes the last dictation kept in the
    /// runtime folder.
    Clear {
        /// Only days before this date (YYYY-MM-DD); without it, everything
        #[arg(long, value_name = "DATE")]
        before: Option<String>,

        /// Don't ask for confirmation (required when not on a terminal)
        #[arg(short, long)]
        yes: bool,
    },

    /// Stop recording dictations until `voxtype history resume`
    ///
    /// Sets history.paused in the config file. A running daemon picks it up
    /// on the next dictation; its --history flags and VOXTYPE_HISTORY_*
    /// variables still win.
    Pause,

    /// Start recording dictations again after `voxtype history pause`
    Resume,

    /// Turn history on (sets history.enabled = true)
    Enable,

    /// Turn history off (sets history.enabled = false); keeps what is stored
    Disable,

    /// Set how many days of history to keep (sets history.retention_days)
    ///
    /// Older days are deleted the next time the daemon prunes: when it starts
    /// and with the first dictation of each day. 0 keeps history forever.
    Retention {
        /// Days to keep; 0 keeps everything
        days: u32,
    },

    /// Whether history is on, paused, and how much it holds
    Status,
}
