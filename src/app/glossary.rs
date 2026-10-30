//! `voxtype glossary`: the words learned from spoken corrections.

use voxtype::config::Config;
use voxtype::text::learn::Learning;
use voxtype::GlossaryAction;

pub(crate) fn run_glossary_command(action: GlossaryAction, config: &Config) -> anyhow::Result<()> {
    let learning = Learning::from_config(&config.text);
    match action {
        GlossaryAction::Learned => {
            let rules = learning.rules();
            if rules.is_empty() {
                println!("No learned words in {}", learning.rules.display());
            }
            for (from, to) in rules {
                println!("{from} = {to}");
            }
        }
        GlossaryAction::Forget { words } => {
            let words = words.join(" ");
            if learning.forget(&words)? {
                println!("Forgot \"{words}\". It stops at the next recording.");
            } else {
                anyhow::bail!(
                    "No learned word \"{words}\" in {}. See `voxtype glossary learned`.",
                    learning.rules.display()
                );
            }
        }
    }
    Ok(())
}
