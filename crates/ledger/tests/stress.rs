//! Many concurrent writers on a real database file. However the writes
//! interleave, fail or roll back, every fiscal year's voucher numbers must
//! run 1..=n with no gap and no duplicate, in the events and in the
//! projection, before and after a rebuild.

use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{DomainError, LedgerEvent, RecordVoucher, Voucher, VoucherLine};
use doris_ledger::{
    Error, close_fiscal_year, correct_voucher, list_fiscal_years, list_vouchers,
    rebuild_projections, record_voucher, record_voucher_in, reopen_fiscal_year, set_account_active,
    trial_balance,
};
use jiff::civil::Date;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const TASKS: usize = 32;
const OPS_PER_TASK: usize = 50;
const TODAY: &str = "2026-10-02";
/// One date in each of the three fiscal years 2024, 2025 and 2026.
const DATES: [&str; 3] = ["2024-06-15", "2025-03-01", "2026-09-30"];
/// Vouchers booked before the stress, per year, for the tasks to correct.
const TARGETS_PER_YEAR: u32 = 3;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn year_start(date: Date) -> Date {
    Date::new(date.year(), 1, 1).unwrap()
}

fn voucher(date: Date, ore: i64, credit_account: u32) -> RecordVoucher {
    RecordVoucher {
        date,
        text: "Stress".into(),
        lines: vec![
            VoucherLine::new(1930, ore, 0).unwrap(),
            VoucherLine::new(credit_account, 0, ore).unwrap(),
        ],
    }
}

/// Successful commits per fiscal year start, as counted by the writers.
type Committed = Arc<Mutex<BTreeMap<Date, u32>>>;

fn count(committed: &Committed, fiscal_year_start: Date) {
    *committed
        .lock()
        .unwrap()
        .entry(fiscal_year_start)
        .or_default() += 1;
}

async fn setup(pool: &SqlitePool, anna: Uuid) -> Uuid {
    let id = doris_company::register_company(
        pool,
        anna,
        NewCompany {
            org_nr: "556016-0680",
            name: "Stress AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: d("2024-01-01"),
            fiscal_year_end: d("2024-12-31"),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap();
    // 1910 Kassa is inactive, so bookings on it must fail.
    set_account_active(pool, id, anna, 1910, false)
        .await
        .unwrap();
    id
}

/// One writer: `OPS_PER_TASK` operations, chosen by their global index `k`.
async fn writer(
    pool: SqlitePool,
    company: Uuid,
    anna: Uuid,
    task: usize,
    committed: Committed,
    corrections: Arc<Mutex<BTreeMap<(Date, u32), u32>>>,
) {
    let today = d(TODAY);
    for op in 0..OPS_PER_TASK {
        let k = task * OPS_PER_TASK + op;
        let date = d(DATES[k % 3]);
        if op == 0 || k % 10 == 3 {
            // Many tasks race to correct the same few vouchers.
            let target_date = if op == 0 { d(DATES[2]) } else { date };
            let number = if op == 0 {
                1
            } else {
                (k / 10) as u32 % TARGETS_PER_YEAR + 1
            };
            let start = year_start(target_date);
            match correct_voucher(&pool, company, anna, start, number, target_date, today).await {
                Ok(r) => {
                    count(&committed, r.fiscal_year_start);
                    *corrections
                        .lock()
                        .unwrap()
                        .entry((start, number))
                        .or_default() += 1;
                }
                Err(Error::Domain(DomainError::AlreadyCorrected)) => {}
                Err(other) => panic!("correction {k}: {other:?}"),
            }
        } else if k.is_multiple_of(7) {
            let cmd = if k.is_multiple_of(2) {
                let mut unbalanced = voucher(date, 100, 3001);
                unbalanced.lines[1].credit = 99;
                unbalanced
            } else {
                voucher(date, 100, 1910)
            };
            match record_voucher(&pool, company, anna, cmd, today).await {
                Err(Error::Domain(
                    DomainError::VoucherUnbalanced | DomainError::AccountInactive,
                )) => {}
                other => panic!("invalid voucher {k} was not refused: {other:?}"),
            }
        } else if k.is_multiple_of(5) {
            // A number is decided, then the transaction is abandoned.
            let mut tx = doris_eventstore::begin(&pool).await.unwrap();
            let abandoned =
                record_voucher_in(&mut tx, company, anna, voucher(date, 100, 3001), today)
                    .await
                    .unwrap_or_else(|e| panic!("abandoned {k}: {e:?}"));
            assert!(abandoned.number >= 1);
            drop(tx);
        } else {
            let r = record_voucher(
                &pool,
                company,
                anna,
                voucher(date, 100 + k as i64, 3001),
                today,
            )
            .await
            .unwrap_or_else(|e| panic!("voucher {k}: {e:?}"));
            count(&committed, r.fiscal_year_start);
        }
    }
}

async fn assert_consistent(
    pool: &SqlitePool,
    company: Uuid,
    anna: Uuid,
    committed: &BTreeMap<Date, u32>,
    corrections: &BTreeMap<(Date, u32), u32>,
) {
    // 1. The events: every year's numbers are exactly 1..=n.
    let mut conn = pool.acquire().await.unwrap();
    let mut numbers: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for event in doris_eventstore::read_all(&mut conn, 0).await.unwrap() {
        if event.stream_id.starts_with("ledger-")
            && let LedgerEvent::VoucherRecorded { number, .. } = event.decode().unwrap()
        {
            numbers
                .entry(event.stream_id.clone())
                .or_default()
                .push(number);
        }
    }
    drop(conn);
    assert_eq!(numbers.len(), 3, "one stream per fiscal year");
    // The raced targets (op 0, and k % 10 == 3) each won by exactly one writer.
    let raced: BTreeMap<(Date, u32), u32> = [
        ("2024-01-01", 1),
        ("2025-01-01", 2),
        ("2026-01-01", 3),
        ("2026-01-01", 1),
    ]
    .map(|(start, number)| ((d(start), number), 1))
    .into();
    assert_eq!(corrections, &raced);
    for (stream, numbers) in &numbers {
        let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
        assert_eq!(numbers, &expected, "{stream}");
    }

    for (&start, &n) in committed {
        // 2. As many vouchers as the writers saw commit.
        let vouchers: Vec<Voucher> = list_vouchers(pool, company, anna, start).await.unwrap();
        assert_eq!(vouchers.len() as u32, n, "{start}");
        // 3. The projection matches the events and every voucher balances.
        let stream = numbers
            .iter()
            .find(|(s, _)| s.ends_with(&start.to_string()))
            .unwrap()
            .1;
        assert_eq!(
            &vouchers.iter().map(|v| v.number).collect::<Vec<_>>(),
            stream
        );
        for v in &vouchers {
            let debit: i64 = v.lines.iter().map(|l| l.debit).sum();
            let credit: i64 = v.lines.iter().map(|l| l.credit).sum();
            assert_eq!(debit, credit, "ver {} in {start}", v.number);
        }
        // 4. Every raced-for voucher was corrected exactly once, no other.
        for target in 1..=TARGETS_PER_YEAR {
            let by = vouchers
                .iter()
                .filter(|v| v.corrects == Some(target))
                .count();
            let expected = corrections.get(&(start, target)).copied().unwrap_or(0);
            assert_eq!(by as u32, expected, "{start} ver {target}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn voucher_numbers_never_gap_or_repeat_under_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("stress.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let company = setup(&pool, anna).await;
    let committed: Committed = Arc::default();
    for date in DATES {
        for _ in 0..TARGETS_PER_YEAR {
            let r = record_voucher(
                &pool,
                company,
                anna,
                voucher(d(date), 1_000, 3001),
                d(TODAY),
            )
            .await
            .unwrap();
            count(&committed, r.fiscal_year_start);
        }
    }
    let corrections = Arc::default();

    let writers: Vec<_> = (0..TASKS)
        .map(|task| {
            tokio::spawn(writer(
                pool.clone(),
                company,
                anna,
                task,
                committed.clone(),
                Arc::clone(&corrections),
            ))
        })
        .collect();
    for w in writers {
        w.await.unwrap();
    }

    let committed = committed.lock().unwrap().clone();
    let corrections = corrections.lock().unwrap().clone();
    assert_eq!(committed.len(), 3);
    assert_consistent(&pool, company, anna, &committed, &corrections).await;
    rebuild_projections(&pool).await.unwrap();
    assert_consistent(&pool, company, anna, &committed, &corrections).await;
}

/// After closers and writers raced on 2024: no voucher sits between a close
/// and the next reopen, numbers run 1..=n, and the projections agree.
async fn assert_closing_consistent(pool: &SqlitePool, company: Uuid, anna: Uuid, start: Date) {
    let mut conn = pool.acquire().await.unwrap();
    let (mut closed, mut numbers) = (false, Vec::new());
    for event in doris_eventstore::read_all(&mut conn, 0).await.unwrap() {
        if !event.stream_id.starts_with("ledger-") {
            continue;
        }
        match event.decode().unwrap() {
            LedgerEvent::VoucherRecorded { number, text, .. } => {
                assert!(!closed, "ver {number} ({text}) recorded in a closed year");
                numbers.push(number);
            }
            LedgerEvent::FiscalYearClosed { .. } => {
                assert!(!closed, "closed twice");
                closed = true;
            }
            LedgerEvent::FiscalYearReopened { .. } => {
                assert!(closed, "reopened an open year");
                closed = false;
            }
            LedgerEvent::OpeningBalancesSet { .. } => {}
            LedgerEvent::AttachmentAdded { .. } => {}
        }
    }
    drop(conn);
    let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
    assert_eq!(numbers, expected);
    let vouchers = list_vouchers(pool, company, anna, start).await.unwrap();
    assert_eq!(
        vouchers.iter().map(|v| v.number).collect::<Vec<_>>(),
        numbers
    );
    let years = list_fiscal_years(pool, company, anna, d(TODAY))
        .await
        .unwrap();
    let status = years.iter().find(|y| y.fiscal_year.start == start).unwrap();
    assert_eq!(status.closed, closed);
    // A closed year's result is on equity, so its resultaträkning nets to 0.
    if closed {
        let rows = trial_balance(pool, company, anna, start).await.unwrap();
        let result: i64 = rows
            .iter()
            .filter(|r| r.account >= 3000)
            .map(|r| r.debit - r.credit)
            .sum();
        assert_eq!(result, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn no_voucher_lands_in_a_closed_year() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("closing.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let company = setup(&pool, anna).await;
    let start = d("2024-01-01");

    let tasks: Vec<_> = (0..16)
        .map(|task| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let today = d(TODAY);
                for op in 0..20 {
                    if task % 4 == 0 {
                        // Closers: close on even ops, reopen on odd ones.
                        let result = if op % 2 == 0 {
                            close_fiscal_year(&pool, company, anna, start, today)
                                .await
                                .map(|_| ())
                        } else {
                            reopen_fiscal_year(&pool, company, anna, start, "Stress", today)
                                .await
                                .map(|_| ())
                        };
                        match result {
                            Ok(())
                            | Err(Error::Domain(
                                DomainError::FiscalYearClosed | DomainError::FiscalYearOpen,
                            )) => {}
                            Err(other) => panic!("closer {task}/{op}: {other:?}"),
                        }
                    } else {
                        let cmd = voucher(d("2024-06-15"), 100 + op as i64, 3001);
                        match record_voucher(&pool, company, anna, cmd, today).await {
                            Ok(_) | Err(Error::Domain(DomainError::FiscalYearClosed)) => {}
                            Err(other) => panic!("writer {task}/{op}: {other:?}"),
                        }
                    }
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }

    assert_closing_consistent(&pool, company, anna, start).await;
    rebuild_projections(&pool).await.unwrap();
    assert_closing_consistent(&pool, company, anna, start).await;
}
