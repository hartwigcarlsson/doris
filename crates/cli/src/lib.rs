//! doris-cli: Doris' books from the command line, for people and agents.

pub mod amount;
pub mod client;
mod commands;
pub mod output;

use clap::{Parser, Subcommand};
use client::{Doris, checked_url};
use output::{Failure, Output};
use std::ffi::OsString;
use std::io::Write;

#[derive(Parser)]
#[command(name = "doris-cli", version, about = "Doris bokföring från terminalen")]
struct Cli {
    /// Answer with one JSON value (and errors as JSON), for programs and agents.
    #[arg(long, global = true)]
    json: bool,
    /// The company: org nr or id. Defaults to DORIS_COMPANY, or the only one.
    #[arg(long, global = true)]
    company: Option<String>,
    /// Run every rule on the server and show what would happen; save nothing.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Area,
}

#[derive(Subcommand)]
enum Area {
    /// Who the token belongs to.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// The companies the token reaches.
    Company {
        #[command(subcommand)]
        action: CompanyAction,
    },
    /// Räkenskapsår.
    Year {
        #[command(subcommand)]
        action: YearAction,
    },
    /// The chart of accounts.
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    Status,
}
#[derive(Subcommand)]
enum CompanyAction {
    List,
    View,
}
#[derive(Subcommand)]
enum YearAction {
    List,
}
#[derive(Subcommand)]
enum AccountAction {
    List,
}

/// The environment doris-cli reads: the token, the server, the company.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub token: Option<String>,
    pub url: Option<String>,
    pub company: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        Self {
            token: var("DORIS_TOKEN"),
            url: var("DORIS_URL"),
            company: var("DORIS_COMPANY"),
        }
    }
}

/// Parses `args` (with the program name first), runs the command and
/// returns the exit code. Writes only to `out` and `err`.
pub async fn run<I, T>(args: I, env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let json = args.iter().any(|a| a == "--json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e)
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = write!(out, "{e}");
            return 0;
        }
        Err(e) => {
            let mut output = Output { json, out, err };
            return output.fail(&Failure::usage(e.to_string().trim().to_owned()));
        }
    };
    let mut output = Output {
        json: cli.json,
        out,
        err,
    };
    match execute(cli, env, &mut output).await {
        Ok(()) => 0,
        Err(failure) => output.fail(&failure),
    }
}

async fn execute(cli: Cli, env: &Env, output: &mut Output<'_>) -> Result<(), Failure> {
    let token = env
        .token
        .clone()
        .ok_or_else(|| Failure::new("missing_token"))?;
    let url = env
        .url
        .as_deref()
        .ok_or_else(|| Failure::new("missing_url"))?;
    let doris =
        Doris::new(checked_url(url)?, token).ok_or_else(|| Failure::new("not_signed_in"))?;
    let context = commands::Context {
        doris,
        company: cli.company.or_else(|| env.company.clone()),
        dry_run: cli.dry_run,
    };
    match cli.command {
        Area::Auth {
            action: AuthAction::Status,
        } => commands::auth::status(&context, output).await,
        Area::Company {
            action: CompanyAction::List,
        } => commands::company::list(&context, output).await,
        Area::Company {
            action: CompanyAction::View,
        } => commands::company::view(&context, output).await,
        Area::Year {
            action: YearAction::List,
        } => commands::year::list(&context, output).await,
        Area::Account {
            action: AccountAction::List,
        } => commands::account::list(&context, output).await,
    }
}

/// The fiscal year `today` falls in, or the one named: `wanted` is a year
/// (matches the start's year) or a start date. `starts` lie in order.
#[allow(dead_code)] // used by `fiscal_year`
pub(crate) fn pick_year(starts: &[String], wanted: Option<&str>, today: &str) -> Option<String> {
    match wanted {
        Some(w) if w.len() == 4 => starts
            .iter()
            .find(|s| s.starts_with(&format!("{w}-")))
            .cloned(),
        Some(w) => starts.iter().find(|s| *s == w).cloned(),
        None => starts.iter().filter(|s| s.as_str() <= today).max().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_year_is_its_start_year_or_its_start_date() {
        let years = ["2025-07-01", "2026-07-01"].map(String::from);
        assert_eq!(
            pick_year(&years, Some("2026"), "2026-10-07"),
            Some("2026-07-01".into())
        );
        assert_eq!(
            pick_year(&years, Some("2025-07-01"), "2026-10-07"),
            Some("2025-07-01".into())
        );
        assert_eq!(
            pick_year(&years, None, "2026-03-01"),
            Some("2025-07-01".into())
        );
        assert_eq!(pick_year(&years, Some("2030"), "2026-10-07"), None);
    }
}
