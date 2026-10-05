//! The first räkenskapsår's end, derived from its start (BFL 3 kap.), so the
//! form only asks for the start. The server validates whatever is sent.
//!
//! It also picks the räkenskapsår a report page shows, and loads the active
//! company's years for the pages that need them.

use crate::active_company::Companies;
use crate::api::{cpb, ledger_api, lpb};
use crate::errors::describe;
use crate::ui::{SELECT_OPTION, Select};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

/// The end of a 12-month räkenskapsår starting on `start` (`YYYY-MM-DD`,
/// the 1st of a month). Enskild firma and handelsbolag must follow the
/// calendar year, so theirs ends 31 December, which shortens a first year
/// that starts mid-year. `None` if `start` isn't the 1st of a month.
pub fn default_end(start: &str, legal_form: cpb::LegalForm) -> Option<String> {
    let (year, month) = match start.as_bytes() {
        [_, _, _, _, b'-', _, _, b'-', b'0', b'1'] => (
            start[..4].parse::<u32>().ok()?,
            start[5..7].parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    if !(1..=12).contains(&month) {
        return None;
    }
    let calendar_year = matches!(
        legal_form,
        cpb::LegalForm::EnskildFirma
            | cpb::LegalForm::Handelsbolag
            | cpb::LegalForm::Kommanditbolag
    );
    // The last day of the month before `month`, one year on.
    let (year, month) = match (calendar_year, month) {
        (true, _) | (false, 1) => (year, 12),
        (false, _) => (year + 1, month - 1),
    };
    Some(format!(
        "{year:04}-{month:02}-{:02}",
        days_in_month(year, month)
    ))
}

/// `preferred` if `years` lists it, otherwise the newest (first) year, or
/// "" when there are none.
pub fn pick_year(years: &[lpb::FiscalYear], preferred: &str) -> String {
    years
        .iter()
        .find(|y| y.start == preferred)
        .or(years.first())
        .map(|y| y.start.clone())
        .unwrap_or_default()
}

/// Whether `start`'s opening balances are still preliminary: the year
/// before it (listed next, years are newest first) is open, so its result
/// is not on equity yet.
pub fn opening_balances_preliminary(years: &[lpb::FiscalYear], start: &str) -> bool {
    years
        .iter()
        .position(|y| y.start == start)
        .and_then(|i| years.get(i + 1))
        .is_some_and(|previous| !previous.closed)
}

/// Whether the listed year starting on `start` is closed.
pub fn is_closed(years: &[lpb::FiscalYear], start: &str) -> bool {
    years.iter().any(|y| y.start == start && y.closed)
}

/// "start – end" for the listed year starting on `start`, or `start` alone.
pub fn period(years: &[lpb::FiscalYear], start: &str) -> String {
    years
        .iter()
        .find(|y| y.start == start)
        .map(|y| format!("{} – {}", y.start, y.end))
        .unwrap_or_else(|| start.to_owned())
}

/// The year that can be closed next: the oldest open one, once it has ended
/// (`end < today`, both `YYYY-MM-DD`).
pub fn closable(years: &[lpb::FiscalYear], today: &str) -> Option<String> {
    years
        .iter()
        .rev()
        .find(|y| !y.closed)
        .filter(|y| y.end.as_str() < today)
        .map(|y| y.start.clone())
}

/// The year that can be reopened: the newest closed one.
pub fn reopenable(years: &[lpb::FiscalYear]) -> Option<String> {
    years.iter().find(|y| y.closed).map(|y| y.start.clone())
}

/// The active company's räkenskapsår (newest first) and the chosen one's
/// start, which is `preferred` if listed, else the newest. Both are cleared
/// and reloaded whenever the active company changes.
pub fn use_fiscal_years(
    preferred: String,
    error: RwSignal<Option<String>>,
) -> (RwSignal<Vec<lpb::FiscalYear>>, RwSignal<String>) {
    let companies = expect_context::<Companies>();
    let years = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    let year = RwSignal::new(String::new());
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's years on screen.
        years.set(Vec::new());
        year.set(String::new());
        error.set(None);
        if company_id.is_empty() {
            return;
        }
        let preferred = preferred.clone();
        spawn_local(async move {
            let result = ledger_api()
                .list_fiscal_years(lpb::ListFiscalYearsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    let chosen = pick_year(&list, &preferred);
                    years.set(list);
                    year.set(chosen);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    (years, year)
}

/// Keeps the chosen year in the URL as `{path}?fy={year}`, replacing the
/// history entry, so Back and reload return to the same year.
pub fn keep_year_in_url(path: String, year: RwSignal<String>) {
    let navigate = use_navigate();
    Effect::new(move |_| {
        let start = year.get();
        if !start.is_empty() {
            navigate(
                &format!("{path}?fy={start}"),
                NavigateOptions {
                    replace: true,
                    scroll: false,
                    ..Default::default()
                },
            );
        }
    });
}

/// The "Räkenskapsår" select over `years`, bound to `year`, for a
/// `PageHeader`: its label is for screen readers only.
#[component]
pub fn FiscalYearSelect(
    years: RwSignal<Vec<lpb::FiscalYear>>,
    year: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class="w-56">
            <Select label="Räkenskapsår" id="fiscal_year" hide_label=true value=year>
                {move || {
                    years
                        .get()
                        .into_iter()
                        .map(|y| view! { <option class=SELECT_OPTION value=y.start.clone()>{format!("{} – {}", y.start, y.end)}</option> })
                        .collect_view()
                }}
            </Select>
        </div>
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::lpb;
    use cpb::LegalForm::*;

    fn years(starts: &[&str]) -> Vec<lpb::FiscalYear> {
        starts
            .iter()
            .map(|s| lpb::FiscalYear {
                start: (*s).into(),
                end: String::new(),
                closed: false,
            })
            .collect()
    }

    fn fy(start: &str, end: &str, closed: bool) -> lpb::FiscalYear {
        lpb::FiscalYear {
            start: start.into(),
            end: end.into(),
            closed,
        }
    }

    /// 2027 and 2026 open, 2025 (the first) closed; newest first.
    fn three_years() -> Vec<lpb::FiscalYear> {
        vec![
            fy("2027-01-01", "2027-12-31", false),
            fy("2026-01-01", "2026-12-31", false),
            fy("2025-01-01", "2025-12-31", true),
        ]
    }

    #[test]
    fn opening_balances_are_preliminary_while_the_year_before_is_open() {
        let ys = three_years();
        assert!(opening_balances_preliminary(&ys, "2027-01-01"));
        assert!(!opening_balances_preliminary(&ys, "2026-01-01"));
        // The first year has no year before it.
        assert!(!opening_balances_preliminary(&ys, "2025-01-01"));
        assert!(!opening_balances_preliminary(&ys, ""));
    }

    #[test]
    fn a_listed_year_is_closed_or_not() {
        let ys = three_years();
        assert!(is_closed(&ys, "2025-01-01"));
        assert!(!is_closed(&ys, "2026-01-01"));
        assert!(!is_closed(&ys, "2024-01-01"));
    }

    #[test]
    fn the_oldest_open_year_can_be_closed_once_it_has_ended() {
        let ys = three_years();
        assert_eq!(closable(&ys, "2027-03-01").as_deref(), Some("2026-01-01"));
        assert_eq!(closable(&ys, "2026-12-31"), None);
        let all_closed: Vec<_> = ys.iter().map(|y| fy(&y.start, &y.end, true)).collect();
        assert_eq!(closable(&all_closed, "2030-01-01"), None);
    }

    #[test]
    fn the_newest_closed_year_can_be_reopened() {
        let mut ys = three_years();
        assert_eq!(reopenable(&ys).as_deref(), Some("2025-01-01"));
        ys[1].closed = true;
        assert_eq!(reopenable(&ys).as_deref(), Some("2026-01-01"));
        assert_eq!(reopenable(&[fy("2025-01-01", "2025-12-31", false)]), None);
    }

    #[test]
    fn pick_year_keeps_a_listed_preference() {
        let ys = years(&["2027-01-01", "2026-01-01"]);
        assert_eq!(pick_year(&ys, "2026-01-01"), "2026-01-01");
    }

    #[test]
    fn pick_year_falls_back_to_the_newest() {
        let ys = years(&["2027-01-01", "2026-01-01"]);
        assert_eq!(pick_year(&ys, ""), "2027-01-01");
        assert_eq!(pick_year(&ys, "nonsense"), "2027-01-01");
        assert_eq!(pick_year(&[], "2026-01-01"), "");
    }

    #[test]
    fn a_year_starting_in_january_ends_31_december() {
        assert_eq!(
            default_end("2026-01-01", Aktiebolag).as_deref(),
            Some("2026-12-31")
        );
    }

    #[test]
    fn a_broken_year_ends_the_day_before_the_same_date_next_year() {
        assert_eq!(
            default_end("2026-05-01", Aktiebolag).as_deref(),
            Some("2027-04-30")
        );
        assert_eq!(
            default_end("2027-03-01", Stiftelse).as_deref(),
            Some("2028-02-29")
        );
        assert_eq!(
            default_end("2026-03-01", Unspecified).as_deref(),
            Some("2027-02-28")
        );
    }

    #[test]
    fn enskild_firma_and_handelsbolag_end_31_december_the_same_year() {
        for form in [EnskildFirma, Handelsbolag, Kommanditbolag] {
            assert_eq!(
                default_end("2026-05-01", form).as_deref(),
                Some("2026-12-31"),
                "{form:?}"
            );
        }
    }

    #[test]
    fn a_start_that_is_not_the_first_of_a_month_has_no_default_end() {
        for start in ["2026-05-02", "", "2026-5-1", "2026-13-01", "not a date"] {
            assert_eq!(default_end(start, Aktiebolag), None, "{start:?}");
        }
    }

    #[test]
    fn a_period_reads_start_to_end() {
        let list = vec![lpb::FiscalYear {
            start: "2025-07-01".into(),
            end: "2026-12-31".into(),
            closed: false,
        }];
        assert_eq!(period(&list, "2025-07-01"), "2025-07-01 – 2026-12-31");
        assert_eq!(period(&list, "2024-07-01"), "2024-07-01");
    }
}
