//! `voxtype history` subcommands.

use clap::Subcommand;

pub const HISTORY_AFTER_HELP: &str = "\
Voxtype keeps the last dictation's text, delivered or not, in the runtime
folder ($XDG_RUNTIME_DIR/voxtype/last-dictation.json, readable by you
alone). It stays until the next dictation replaces it, and is gone at
logout. Bind `voxtype history copy` to a key to get the text back when a
paste did not land, or when restore_clipboard put the old clipboard back.

EXAMPLES:
  voxtype history                      Show the last dictation
  voxtype history copy                 Copy the last dictation to the clipboard
  voxtype history copy --json          The same, with a JSON result

KEY BINDINGS:
  KDE Plasma:  System Settings > Shortcuts > Add Command: voxtype history copy
  Hyprland:    bind = SUPER SHIFT, V, exec, voxtype history copy
  Sway:        bindsym $mod+Shift+v exec voxtype history copy";

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum HistoryAction {
    /// Copy a dictation's text to the clipboard (the most recent when no ID)
    ///
    /// Copies the last dictation's final text, also when its delivery failed
    /// or a paste did not land, so it can still be recovered.
    Copy {
        /// `last` (the default) or 1; earlier dictations need history
        id: Option<String>,
    },
}
