//! Commands, and the lookups they share.

pub mod account;
pub mod auth;
pub mod company;
pub mod report;
pub mod skill;
pub mod ver;
pub mod year;

use crate::client::Doris;
use crate::output::{Failure, Output};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::messages::message;
use serde_json::{Value, json};

pub struct Context {
    pub doris: Doris,
    pub company: Option<String>,
    pub dry_run: bool,
}

impl Context {
    /// A reading command prints `value`, flagging --dry-run (which changes
    /// nothing there) in objects; lists stay lists.
    pub fn print(&self, output: &mut Output<'_>, mut value: Value, mut text: String) {
        if self.dry_run {
            if let Some(object) = value.as_object_mut() {
                object.insert("dry_run".into(), json!(true));
            }
            text.push_str("(--dry-run: kommandot ändrar ingenting.)\n");
        }
        output.print(value, &text);
    }
}

fn digits(s: &str) -> String {
    s.chars().filter(|c| *c != '-').collect()
}

/// The company the command is about: --company / DORIS_COMPANY (org nr,
/// with or without the dash, or id), or the token's only company.
pub async fn company(context: &Context) -> Result<cpb::CompanySummary, Failure> {
    let mut client = context.doris.companies();
    let companies = client
        .list_companies(context.doris.request(cpb::ListCompaniesRequest {}))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .companies;
    match &context.company {
        Some(wanted) => companies
            .into_iter()
            .find(|c| c.id == *wanted || digits(&c.org_nr) == digits(wanted))
            .ok_or_else(|| Failure::new("company_not_found")),
        None => match companies.len() {
            0 => Err(Failure::new("company_not_found")),
            1 => Ok(companies.into_iter().next().expect("one company")),
            _ => {
                let lines: Vec<String> = companies
                    .iter()
                    .map(|c| format!("{}  {}", c.org_nr, c.name))
                    .collect();
                Err(Failure {
                    code: "company_ambiguous".into(),
                    message: format!("{}\n{}", message("company_ambiguous"), lines.join("\n")),
                    exit: 2,
                    details: None,
                })
            }
        },
    }
}

/// The fiscal year's start date: --year (a year or a start date), or the
/// year today (in Sweden) falls in.
pub async fn fiscal_year(
    context: &Context,
    company_id: &str,
    wanted: Option<&str>,
) -> Result<String, Failure> {
    let years = context
        .doris
        .ledger()
        .list_fiscal_years(context.doris.request(lpb::ListFiscalYearsRequest {
            company_id: company_id.into(),
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .fiscal_years;
    let starts: Vec<String> = years.into_iter().map(|y| y.start).collect();
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::get("Europe/Stockholm").expect("bundled tz database"))
        .date()
        .to_string();
    crate::pick_year(&starts, wanted, &today).ok_or_else(|| Failure::new("fiscal_year_not_found"))
}

/// A year is `ÅÅÅÅ` or a real `ÅÅÅÅ-MM-DD`; anything else is a usage
/// error, found before any call. `label` is how the caller names it.
pub(crate) fn check_year(year: Option<&str>, label: &str) -> Result<(), Failure> {
    let Some(y) = year else { return Ok(()) };
    let ok = (y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit()))
        || (y.len() == 10 && y.parse::<jiff::civil::Date>().is_ok());
    ok.then_some(()).ok_or_else(|| {
        Failure::usage(format!(
            "Ogiltigt {label} \"{y}\": skriv ett år (2026) eller ett startdatum (2026-07-01)."
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::check_year;

    #[test]
    fn the_year_check_names_the_field_it_was_given() {
        assert!(check_year(Some("2026"), "--year").is_ok());
        assert!(check_year(Some("2026-07-01"), "year").is_ok());
        let f = check_year(Some("26"), "year").unwrap_err();
        assert_eq!(f.code, "usage");
        assert!(
            f.message.starts_with("Ogiltigt year \"26\""),
            "{}",
            f.message
        );
    }
}
