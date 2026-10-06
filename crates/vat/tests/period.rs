use doris_company::domain::FiscalYear;
use doris_vat::period::{VatPeriod, VatPeriodKind::*, due_date, periods};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn year(start: &str, end: &str) -> FiscalYear {
    FiscalYear { start: d(start), end: d(end) }
}

fn spans(list: &[VatPeriod]) -> Vec<(String, String)> {
    list.iter().map(|p| (p.start.to_string(), p.end.to_string())).collect()
}

#[test]
fn a_calendar_year_has_twelve_months_four_quarters_or_one_year() {
    let y = year("2026-01-01", "2026-12-31");
    let months = periods(y, Monthly);
    assert_eq!(months.len(), 12);
    assert_eq!(spans(&months[1..2]), [("2026-02-01".into(), "2026-02-28".into())]);
    assert_eq!(
        spans(&periods(y, Quarterly)),
        [
            ("2026-01-01".into(), "2026-03-31".into()),
            ("2026-04-01".into(), "2026-06-30".into()),
            ("2026-07-01".into(), "2026-09-30".into()),
            ("2026-10-01".into(), "2026-12-31".into()),
        ]
    );
    assert_eq!(spans(&periods(y, Yearly)), [("2026-01-01".into(), "2026-12-31".into())]);
    assert!(periods(y, NotRegistered).is_empty());
}

#[test]
fn quarters_follow_the_calendar_in_a_broken_year() {
    // May 2026 – April 2027: the quarters ending June, September, December
    // and March. April–June 2027 belongs to the next year.
    let quarters = periods(year("2026-05-01", "2027-04-30"), Quarterly);
    assert_eq!(
        spans(&quarters),
        [
            ("2026-04-01".into(), "2026-06-30".into()),
            ("2026-07-01".into(), "2026-09-30".into()),
            ("2026-10-01".into(), "2026-12-31".into()),
            ("2027-01-01".into(), "2027-03-31".into()),
        ]
    );
    assert_eq!(periods(year("2026-05-01", "2027-04-30"), Yearly)[0].end, d("2027-04-30"));
}

#[test]
fn periods_have_a_code_and_a_swedish_label() {
    let q3 = VatPeriod { start: d("2026-07-01"), end: d("2026-09-30") };
    assert_eq!((q3.code(), q3.label()), ("202609".into(), "juli–september 2026".into()));
    let may = VatPeriod { start: d("2026-05-01"), end: d("2026-05-31") };
    assert_eq!(may.label(), "maj 2026");
    let broken = VatPeriod { start: d("2026-05-01"), end: d("2027-04-30") };
    assert_eq!(broken.label(), "maj 2026–april 2027");
}

fn due(start: &str, end: &str, kind: doris_vat::period::VatPeriodKind) -> Option<String> {
    due_date(VatPeriod { start: d(start), end: d(end) }, kind).map(|d| d.to_string())
}

#[test]
fn the_declaration_is_due_the_12th_of_the_second_month_or_the_17th_in_january_and_august() {
    assert_eq!(due("2026-07-01", "2026-09-30", Quarterly).as_deref(), Some("2026-11-12"));
    assert_eq!(due("2026-04-01", "2026-06-30", Quarterly).as_deref(), Some("2026-08-17"));
    assert_eq!(due("2026-03-01", "2026-03-31", Monthly).as_deref(), Some("2026-05-12"));
    // 17 January 2027 is a Sunday.
    assert_eq!(due("2026-11-01", "2026-11-30", Monthly).as_deref(), Some("2027-01-18"));
    // 12 April 2026 is a Sunday.
    assert_eq!(due("2026-02-01", "2026-02-28", Monthly).as_deref(), Some("2026-04-13"));
    assert_eq!(due("2026-01-01", "2026-12-31", Yearly), None);
}

#[test]
fn easter_holidays_push_the_date_to_the_next_workday() {
    // Annandag påsk 2004-04-12.
    assert_eq!(due("2004-02-01", "2004-02-29", Monthly).as_deref(), Some("2004-04-13"));
    // Långfredag 2047-04-12, then a weekend and annandag påsk on the 15th.
    assert_eq!(due("2047-02-01", "2047-02-28", Monthly).as_deref(), Some("2047-04-16"));
    // Kristi himmelsfärdsdag 2067-05-12.
    assert_eq!(due("2067-03-01", "2067-03-31", Monthly).as_deref(), Some("2067-05-13"));
}
