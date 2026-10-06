//! The momsdeklaration of a company: the redovisningsperiod per
//! räkenskapsår, each period's boxes from the ledger, the file for
//! Skatteverket, and the settlement voucher booked when a period is marked
//! submitted.

pub mod domain;
pub mod eskd;
pub mod period;

use domain::{AccountSaldo, Boxes, DomainError, Submission, Vat, VatEvent, VatStatus};
use doris_company::domain::{Company, FiscalYear};
use doris_eventstore::{Metadata, NewEvent, RecordedEvent};
use doris_ledger::{VatAccountTotal, VoucherRef};
use jiff::civil::Date;
use period::{VatPeriod, VatPeriodKind};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

const VAT_STREAM: &str = "vat-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// The ledger refused (closed year, inactive account, no such year…).
    #[error(transparent)]
    Ledger(#[from] doris_ledger::Error),
    #[error(transparent)]
    Store(#[from] doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<doris_company::Error> for Error {
    fn from(err: doris_company::Error) -> Self {
        match err {
            doris_company::Error::Store(err) => Error::Store(err),
            _ => Error::NotFound,
        }
    }
}

fn vat_stream(company_id: Uuid) -> String {
    format!("{VAT_STREAM}{company_id}")
}

/// Membership, the company, its VAT state, the stream's version and its
/// recorded events (for who submitted what, and when).
async fn load(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
) -> Result<(Company, Vat, i64, Vec<RecordedEvent>)> {
    let company = doris_company::get_company_in(conn, company_id, actor).await?;
    let recorded = doris_eventstore::load(conn, &vat_stream(company_id)).await?;
    let events = recorded
        .iter()
        .map(|e| e.decode::<VatEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    let version = doris_eventstore::stream_version(conn, &vat_stream(company_id)).await?;
    Ok((company, Vat::from_events(&events), version, recorded))
}

/// The räkenskapsår that starts on `start`, or the ledger's
/// `fiscal_year_not_found`.
fn fiscal_year(company: &Company, start: Date) -> Result<FiscalYear> {
    let year = company.first_fiscal_year.containing(start);
    if year.start != start {
        return Err(doris_ledger::Error::Domain(
            doris_ledger::domain::DomainError::FiscalYearNotFound,
        )
        .into());
    }
    Ok(year)
}

/// The periods of `year`, the first starting after the year before's last
/// (`period::periods_after`).
fn year_periods(
    company: &Company,
    vat: &Vat,
    year: FiscalYear,
    kind: VatPeriodKind,
) -> Vec<VatPeriod> {
    let previous = (year.start > company.first_fiscal_year.start).then(|| {
        let before = company
            .first_fiscal_year
            .containing(year.start.yesterday().expect("far from the date limits"));
        (before, vat.kind(before.start))
    });
    period::periods_after(year, kind, previous)
}

/// The period ending `period_end`, of the year its last month is in.
fn resolve(company: &Company, vat: &Vat, period_end: Date) -> Result<(VatPeriodKind, VatPeriod)> {
    let year = company.first_fiscal_year.containing(period_end);
    let kind = vat.kind(year.start);
    if kind == VatPeriodKind::NotRegistered {
        return Err(DomainError::VatNotRegistered.into());
    }
    let period = year_periods(company, vat, year, kind)
        .into_iter()
        .find(|p| p.end == period_end)
        .ok_or(DomainError::InvalidVatPeriod)?;
    Ok((kind, period))
}

async fn totals_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    vat: &Vat,
    period: VatPeriod,
) -> Result<Vec<VatAccountTotal>> {
    Ok(
        doris_ledger::vat_box_totals_in(
            conn,
            company_id,
            period.start,
            period.end,
            &vat.vouchers(),
        )
        .await?,
    )
}

async fn append(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    expected_version: i64,
    events: &[VatEvent],
    actor: Uuid,
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: Some(actor.to_string()),
    };
    doris_eventstore::append(
        conn,
        &vat_stream(company_id),
        expected_version,
        &new_events,
        &metadata,
    )
    .await?;
    Ok(())
}

pub async fn set_vat_period(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    kind: VatPeriodKind,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, vat, version, _) = load(&mut tx, company_id, actor).await?;
    let year = fiscal_year(&company, fiscal_year_start)?;
    let events = domain::set_vat_period(&vat, year, kind)?;
    append(&mut tx, company_id, version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeriodSummary {
    pub period: VatPeriod,
    pub status: VatStatus,
    pub due_date: Option<Date>,
    /// Box 49, kronor.
    pub vat_due: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VatYear {
    pub kind: VatPeriodKind,
    /// A period of the year is submitted, so the kind cannot change.
    pub locked: bool,
    pub periods: Vec<PeriodSummary>,
}

async fn summary(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    vat: &Vat,
    kind: VatPeriodKind,
    period: VatPeriod,
    today: Date,
    corrected: &HashSet<VoucherRef>,
) -> Result<(PeriodSummary, Vec<VatAccountTotal>, Vec<AccountSaldo>)> {
    let totals = totals_in(conn, company_id, vat, period).await?;
    let accounts = domain::saldos(&totals);
    let summary = PeriodSummary {
        period,
        status: domain::status(vat, period, today, &accounts, corrected),
        due_date: period::due_date(period, kind),
        vat_due: domain::boxes(&accounts).vat_due,
    };
    Ok((summary, totals, accounts))
}

// ponytail: one ledger query per period (at most 13 a year); one grouped
// query when that shows.
pub async fn list_vat_returns(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    today: Date,
) -> Result<VatYear> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, _) = load(&mut conn, company_id, actor).await?;
    let year = fiscal_year(&company, fiscal_year_start)?;
    let kind = vat.kind(year.start);
    let corrected = doris_ledger::corrected_vouchers_in(&mut conn, company_id).await?;
    let mut periods = Vec::new();
    for period in year_periods(&company, &vat, year, kind) {
        periods.push(
            summary(&mut conn, company_id, &vat, kind, period, today, &corrected)
                .await?
                .0,
        );
    }
    Ok(VatYear {
        kind,
        locked: domain::is_locked(&vat, year),
        periods,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionRecord {
    pub submission: Submission,
    pub submitted_at: String,
    pub submitted_by: Option<Uuid>,
    /// Its settlement voucher has been corrected.
    pub corrected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VatReturnView {
    pub summary: PeriodSummary,
    pub kind: VatPeriodKind,
    pub org_nr: String,
    pub vat_number: String,
    pub totals: Vec<VatAccountTotal>,
    pub boxes: Boxes,
    /// Box 49 in öre before rounding.
    pub booked_vat: i64,
    pub fingerprint: String,
    pub submissions: Vec<SubmissionRecord>,
}

pub async fn get_vat_return(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period_end: Date,
    today: Date,
) -> Result<VatReturnView> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, recorded) = load(&mut conn, company_id, actor).await?;
    let (kind, period) = resolve(&company, &vat, period_end)?;
    let corrected = doris_ledger::corrected_vouchers_in(&mut conn, company_id).await?;
    let (summary, totals, accounts) =
        summary(&mut conn, company_id, &vat, kind, period, today, &corrected).await?;
    let mut submissions = Vec::new();
    for event in &recorded {
        if let VatEvent::VatReturnSubmitted(submission) = event.decode::<VatEvent>()?
            && submission.period_end == period_end
        {
            submissions.push(SubmissionRecord {
                corrected: submission.voucher.is_some_and(|v| corrected.contains(&v)),
                submitted_at: event.recorded_at.clone(),
                submitted_by: event.metadata.actor.as_deref().and_then(|a| a.parse().ok()),
                submission,
            });
        }
    }
    Ok(VatReturnView {
        summary,
        kind,
        org_nr: company.org_nr.formatted(),
        vat_number: eskd::vat_number(&company.org_nr),
        boxes: domain::boxes(&accounts),
        booked_vat: domain::booked_vat(&accounts),
        fingerprint: domain::fingerprint(period.end, &accounts),
        totals,
        submissions,
    })
}

/// The file to upload, its name, and the fingerprint of what it declares.
pub async fn export_vat_file(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period_end: Date,
    today: Date,
) -> Result<(String, String, String)> {
    let mut conn = pool.acquire().await?;
    let (company, vat, _, _) = load(&mut conn, company_id, actor).await?;
    let (_, period) = resolve(&company, &vat, period_end)?;
    if period.end >= today {
        return Err(DomainError::VatPeriodNotEnded.into());
    }
    let accounts = domain::saldos(&totals_in(&mut conn, company_id, &vat, period).await?);
    let xml = eskd::eskd_xml(&company.org_nr, period, &domain::boxes(&accounts));
    Ok((
        eskd::file_name(&company.org_nr, period),
        xml,
        domain::fingerprint(period.end, &accounts),
    ))
}

/// Records the period as submitted and books its settlement voucher (dated
/// the period's last day) in one transaction: both, or nothing and no
/// voucher number used up. `fingerprint` is that of the file downloaded or
/// the declaration shown.
pub async fn mark_vat_return_submitted(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period_end: Date,
    fingerprint: &str,
    today: Date,
) -> Result<Option<VoucherRef>> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, vat, version, _) = load(&mut tx, company_id, actor).await?;
    let (kind, period) = resolve(&company, &vat, period_end)?;
    let accounts = domain::saldos(&totals_in(&mut tx, company_id, &vat, period).await?);
    let corrected = doris_ledger::corrected_vouchers_in(&mut tx, company_id).await?;
    let domain::Prepared {
        lines,
        mut submission,
    } = domain::submit(&vat, period, kind, today, accounts, fingerprint, &corrected)?;
    if !lines.is_empty() {
        let cmd = doris_ledger::domain::RecordVoucher {
            date: period.end,
            text: format!("Momsavräkning {}", period.label()),
            lines,
        };
        submission.voucher =
            Some(doris_ledger::record_voucher_in(&mut tx, company_id, actor, cmd, today).await?);
    }
    let voucher = submission.voucher;
    append(
        &mut tx,
        company_id,
        version,
        &[VatEvent::VatReturnSubmitted(submission)],
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(voucher)
}
