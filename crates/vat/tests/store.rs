#![allow(clippy::inconsistent_digit_grouping)]
use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{RecordVoucher, VoucherLine};
use doris_vat::domain::{DomainError, VatStatus};
use doris_vat::period::VatPeriodKind;
use doris_vat::{Error, export_vat_file, get_vat_return, list_vat_returns, set_vat_period};
use jiff::civil::Date;
use sqlx::SqlitePool;
use uuid::Uuid;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

const TODAY: &str = "2026-10-06";

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// Exempel AB, first räkenskapsår 2026, faktureringsmetoden.
async fn company(pool: &SqlitePool, owner: Uuid, method: AccountingMethod) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: d("2026-01-01"),
            fiscal_year_end: d("2026-12-31"),
            accounting_method: method,
        },
    )
    .await
    .unwrap()
}

async fn book(pool: &SqlitePool, id: Uuid, user: Uuid, date: &str, lines: &[(u32, i64, i64)]) {
    let cmd = RecordVoucher {
        date: d(date),
        text: "Underlag".into(),
        lines: lines.iter().map(|&(a, dr, cr)| VoucherLine::new(a, dr, cr).unwrap()).collect(),
    };
    doris_ledger::record_voucher(pool, id, user, cmd, d(TODAY)).await.unwrap();
}

/// A sale of 10 000,40 + 25 % VAT and a purchase with 400,30 input VAT in Q3.
async fn q3_books(pool: &SqlitePool, id: Uuid, user: Uuid) {
    book(pool, id, user, "2026-08-10", &[(1510, 12_500_90, 0), (3001, 0, 10_000_40), (2611, 0, 2_500_50)]).await;
    book(pool, id, user, "2026-09-15", &[(4010, 1_601_20, 0), (2640, 400_30, 0), (2440, 0, 2_001_50)]).await;
}

#[tokio::test]
async fn a_year_lists_its_quarters_with_status_due_date_and_box_49() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert_eq!((year.kind, year.locked), (VatPeriodKind::Quarterly, false));
    let rows: Vec<(String, VatStatus, Option<String>, i64)> = year
        .periods
        .iter()
        .map(|p| (p.period.code(), p.status, p.due_date.map(|d| d.to_string()), p.vat_due))
        .collect();
    assert_eq!(
        rows,
        [
            ("202603".into(), VatStatus::ToSubmit, Some("2026-05-12".into()), 0),
            ("202606".into(), VatStatus::ToSubmit, Some("2026-08-17".into()), 0),
            ("202609".into(), VatStatus::ToSubmit, Some("2026-11-12".into()), 2_100),
            ("202612".into(), VatStatus::InProgress, Some("2027-02-12".into()), 0),
        ]
    );
}

#[tokio::test]
async fn a_period_shows_its_boxes_accounts_and_file() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    q3_books(&pool, id, anna).await;
    let view = get_vat_return(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!((view.boxes.get(5), view.boxes.get(10), view.boxes.get(48), view.boxes.vat_due), (10_000, 2_500, 400, 2_100));
    assert_eq!(view.booked_vat, 2_100_20);
    assert_eq!((view.org_nr.as_str(), view.vat_number.as_str()), ("556016-0680", "SE556016068001"));
    assert_eq!(view.totals.iter().map(|t| t.number).collect::<Vec<_>>(), [2611, 2640, 3001]);
    assert!(view.submissions.is_empty());

    let (name, xml, print) = export_vat_file(&pool, id, anna, d("2026-09-30"), d(TODAY)).await.unwrap();
    assert_eq!(name, "moms_5560160680_202609.xml");
    assert!(xml.contains("<MomsBetala>2100</MomsBetala>"));
    assert_eq!(print, view.fingerprint);

    let early = export_vat_file(&pool, id, anna, d("2026-12-31"), d(TODAY)).await;
    assert!(matches!(early, Err(Error::Domain(DomainError::VatPeriodNotEnded))));
    let no_such = get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await;
    assert!(matches!(no_such, Err(Error::Domain(DomainError::InvalidVatPeriod))));
    let stranger = get_vat_return(&pool, id, Uuid::new_v4(), d("2026-09-30"), d(TODAY)).await;
    assert!(matches!(stranger, Err(Error::NotFound)));
}

#[tokio::test]
async fn monthly_and_not_registered_change_the_periods() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, AccountingMethod::Invoice).await;
    set_vat_period(&pool, id, anna, d("2026-01-01"), VatPeriodKind::Monthly).await.unwrap();
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert_eq!((year.kind, year.periods.len()), (VatPeriodKind::Monthly, 12));
    assert!(get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await.is_ok());

    set_vat_period(&pool, id, anna, d("2026-01-01"), VatPeriodKind::NotRegistered).await.unwrap();
    let year = list_vat_returns(&pool, id, anna, d("2026-01-01"), d(TODAY)).await.unwrap();
    assert!(year.periods.is_empty());
    let refused = get_vat_return(&pool, id, anna, d("2026-08-31"), d(TODAY)).await;
    assert!(matches!(refused, Err(Error::Domain(DomainError::VatNotRegistered))));

    let not_a_year = set_vat_period(&pool, id, anna, d("2026-02-01"), VatPeriodKind::Monthly).await;
    assert!(matches!(not_a_year, Err(Error::Ledger(_))));
}
