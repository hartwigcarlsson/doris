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
    /// Svara med exakt ett JSON-värde (fel också som JSON), för program och agenter.
    #[arg(long, global = true)]
    json: bool,
    /// Företaget: organisationsnummer eller id. Standard är DORIS_COMPANY, annars det enda företaget.
    #[arg(long, global = true)]
    company: Option<String>,
    /// Kör alla regler på servern och visa vad som skulle hända; spara ingenting.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Area,
}

#[derive(Subcommand)]
enum Area {
    /// Vem token tillhör.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Företagen som token når.
    Company {
        #[command(subcommand)]
        action: CompanyAction,
    },
    /// Räkenskapsår.
    Year {
        #[command(subcommand)]
        action: YearAction,
    },
    /// Kontoplanen.
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Verifikationer.
    Ver {
        #[command(subcommand)]
        action: commands::ver::VerAction,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Visa vem token tillhör.
    Status,
}
#[derive(Subcommand)]
enum CompanyAction {
    /// Lista företagen.
    List,
    /// Visa ett företag.
    View,
}
#[derive(Subcommand)]
enum YearAction {
    /// Lista räkenskapsåren.
    List,
}
#[derive(Subcommand)]
enum AccountAction {
    /// Lista kontona.
    List,
}

/// The environment doris-cli reads: the token, the server, the company.
#[derive(Clone, Default)]
pub struct Env {
    pub token: Option<String>,
    pub url: Option<String>,
    pub company: Option<String>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = self.token.as_ref().map(|_| "<redacted>");
        f.debug_struct("Env")
            .field("token", &token)
            .field("url", &self.url)
            .field("company", &self.company)
            .finish()
    }
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
        Area::Ver { action } => {
            use commands::ver::{self, VerAction};
            match action {
                VerAction::List { year } => ver::list(&context, output, year.as_deref()).await,
                VerAction::View { number, year } => {
                    ver::view(&context, output, number, year.as_deref()).await
                }
                VerAction::New(args) => ver::new(&context, output, &args).await,
                VerAction::Correct { number, date, year } => {
                    ver::correct(&context, output, number, &date, year.as_deref()).await
                }
            }
        }
    }
}

/// The fiscal year `today` falls in, or the one named: `wanted` is a year
/// (matches the start's year) or a start date. `starts` lie in order.
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
    fn env_debug_hides_the_token() {
        let env = Env {
            token: Some("doris_secret".into()),
            url: Some("https://x".into()),
            company: None,
        };
        let shown = format!("{env:?}");
        assert!(!shown.contains("doris_secret"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }

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
