//! The agent skill doris-bookkeeping, built into the binary so it always
//! matches this version of the commands.

use crate::output::{Failure, Output};
use clap::Subcommand;
use serde_json::json;
use std::path::{Path, PathBuf};

/// The skill's folder name, as agents and `npx skills` know it.
pub const NAME: &str = "doris-bookkeeping";
pub const SKILL: &str = include_str!("../../../../skills/doris-bookkeeping/SKILL.md");
pub const REFERENCE: &str = include_str!("../../../../skills/doris-bookkeeping/reference.md");

#[derive(Subcommand)]
pub enum SkillAction {
    /// Skriv ut skillen (SKILL.md), t.ex. för en systemprompt.
    Show,
    /// Installera skillen i en mapp för agenters skills (standard ~/.agents/skills).
    Install {
        /// Mappen skillen läggs i, som <MAPP>/doris-bookkeeping.
        dir: Option<PathBuf>,
    },
}

/// Where skills shared by several agents live: `~/.agents/skills`.
pub fn default_dir(home: Option<&str>) -> Option<PathBuf> {
    home.map(|home| Path::new(home).join(".agents").join("skills"))
}

pub fn run(action: &SkillAction, output: &mut Output<'_>) -> Result<(), Failure> {
    match action {
        SkillAction::Show => {
            let value = json!({ "name": NAME, "skill": SKILL, "reference": REFERENCE });
            output.print(value, SKILL);
            Ok(())
        }
        SkillAction::Install { dir } => {
            let home = std::env::var("HOME").ok();
            let dir = match dir {
                Some(dir) => dir.clone(),
                None => default_dir(home.as_deref())
                    .ok_or_else(|| Failure::usage("Ange en mapp: doris-cli skill install MAPP."))?,
            };
            let folder = dir.join(NAME);
            let written = std::fs::create_dir_all(&folder)
                .and_then(|()| std::fs::write(folder.join("SKILL.md"), SKILL))
                .and_then(|()| std::fs::write(folder.join("reference.md"), REFERENCE));
            written.map_err(|e| {
                Failure::usage(format!("Kunde inte skriva {}: {e}", folder.display()))
            })?;
            let value = json!({
                "installed": folder.to_string_lossy(),
                "files": ["SKILL.md", "reference.md"],
            });
            let text = format!("Installerade {NAME} i {}.\n", folder.display());
            output.print(value, &text);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Env, run};
    use clap::CommandFactory;
    use std::collections::BTreeSet;

    async fn cli(args: &[&str]) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut all = vec!["doris-cli"];
        all.extend_from_slice(args);
        let code = run(all, &Env::default(), &mut out, &mut err).await;
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("doris-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn show_prints_the_skill_without_a_token() {
        let (code, out, err) = cli(&["skill", "show"]).await;

        assert_eq!(code, 0, "{err}");
        assert_eq!(out, SKILL);
        assert!(out.starts_with("---\nname: doris-bookkeeping\n"));
    }

    #[tokio::test]
    async fn install_writes_the_skill_folder() {
        let dir = scratch("install");

        let (code, out, err) = cli(&["skill", "install", dir.to_str().unwrap(), "--json"]).await;

        assert_eq!(code, 0, "{err}");
        let folder = dir.join("doris-bookkeeping");
        assert_eq!(
            std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
            SKILL
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("reference.md")).unwrap(),
            REFERENCE
        );
        let answer: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(answer["installed"], folder.to_str().unwrap());
        assert_eq!(
            answer["files"],
            serde_json::json!(["SKILL.md", "reference.md"])
        );
        // Installing again replaces the files: an upgrade is the same command.
        let (again, _, _) = cli(&["skill", "install", dir.to_str().unwrap()]).await;
        assert_eq!(again, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_default_folder_is_the_shared_agents_one() {
        assert_eq!(
            default_dir(Some("/home/anna")),
            Some(std::path::PathBuf::from("/home/anna/.agents/skills"))
        );
        assert_eq!(default_dir(None), None);
    }

    /// Every `area action` the CLI has, from clap.
    fn commands() -> BTreeSet<String> {
        Cli::command()
            .get_subcommands()
            .flat_map(|area| {
                area.get_subcommands()
                    .map(|action| format!("{} {}", area.get_name(), action.get_name()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Every long flag the CLI has, on any command.
    fn flags() -> BTreeSet<String> {
        fn walk(command: &clap::Command, into: &mut BTreeSet<String>) {
            for arg in command.get_arguments() {
                if let Some(long) = arg.get_long() {
                    into.insert(format!("--{long}"));
                }
            }
            for sub in command.get_subcommands() {
                walk(sub, into);
            }
        }
        let mut all = BTreeSet::from(["--help".to_owned(), "--version".to_owned()]);
        walk(&Cli::command(), &mut all);
        all
    }

    /// `--word` tokens in a text.
    fn mentioned_flags(text: &str) -> BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|word| {
                word.starts_with("--") && word[2..].starts_with(|c: char| c.is_ascii_lowercase())
            })
            .map(String::from)
            .collect()
    }

    #[test]
    fn the_skill_names_only_flags_the_cli_has() {
        let known = flags();
        for (file, text) in [("SKILL.md", SKILL), ("reference.md", REFERENCE)] {
            let unknown: Vec<_> = mentioned_flags(text).difference(&known).cloned().collect();
            assert!(
                unknown.is_empty(),
                "{file} mentions {unknown:?}, which doris-cli doesn't have"
            );
        }
    }

    #[test]
    fn the_skill_stands_alone_outside_the_repository() {
        for (file, text) in [("SKILL.md", SKILL), ("reference.md", REFERENCE)] {
            for repo_path in ["crates/", "README", ".claude/"] {
                assert!(
                    !text.contains(repo_path),
                    "{file} points into the repository ({repo_path})"
                );
            }
        }
        assert!(
            SKILL.contains("(reference.md)"),
            "SKILL.md should point to its reference"
        );
    }

    #[test]
    fn the_reference_covers_every_command() {
        let missing: Vec<_> = commands()
            .into_iter()
            .filter(|command| !REFERENCE.contains(&format!("`{command}")))
            .collect();
        assert!(
            missing.is_empty(),
            "reference.md has no section for {missing:?}"
        );
    }
}
