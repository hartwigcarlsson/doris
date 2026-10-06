#![allow(clippy::inconsistent_digit_grouping)] // kronor_öre, as the pages show amounts

use doris_ledger::vat_box::VatBox;
use doris_vat::domain::{AccountSaldo, booked_vat, boxes, fingerprint};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(account: u16, vat_box: u32, saldo: i64) -> AccountSaldo {
    AccountSaldo { account, vat_box: VatBox::parse(vat_box).unwrap(), saldo }
}

#[test]
fn boxes_count_sales_as_credits_and_purchases_as_debits_in_whole_kronor() {
    let accounts = [
        a(2611, 10, -9_500_037),   // output VAT 25 %
        a(2640, 48, 2_461_099),    // input VAT
        a(3001, 5, -38_000_000),
        a(3002, 5, -3_230_040),
        a(4535, 21, 480_000),      // EU service purchase
        a(2614, 30, -120_000),     // its output VAT
        a(2645, 48, 120_000),      // and its input VAT
    ];
    let b = boxes(&accounts);
    assert_eq!(
        [b.get(5), b.get(10), b.get(21), b.get(30), b.get(48)],
        [412_300, 95_000, 4_800, 1_200, 25_810]
    );
    // 49 from the rounded boxes: 95 000 + 1 200 − 25 810.
    assert_eq!(b.vat_due, 70_390);
    assert_eq!(b.get(6), 0);
    assert!(b.amounts.iter().all(|(_, kr)| *kr != 0));
    // Exactly booked: 95 000,37 + 1 200,00 − 25 810,99 = 70 389,38.
    assert_eq!(booked_vat(&accounts), 7_038_938);
}

#[test]
fn ore_are_struck_off_toward_zero_also_for_negative_boxes() {
    // More credited than sold: a debit saldo on sales and output VAT.
    let b = boxes(&[a(3001, 5, 100_099), a(2611, 10, 25_075)]);
    assert_eq!((b.get(5), b.get(10), b.vat_due), (-1_000, -250, -250));
}

#[test]
fn the_fingerprint_follows_the_accounts_and_their_boxes() {
    let one = fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]);
    assert_eq!(one.len(), 64);
    assert_eq!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -100)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 10, -101)]));
    assert_ne!(one, fingerprint(d("2026-09-30"), &[a(2611, 11, -100)]));
    assert_ne!(one, fingerprint(d("2026-06-30"), &[a(2611, 10, -100)]));
}

use doris_company::domain::FiscalYear;
use doris_ledger::VoucherRef;
use doris_ledger::domain::VoucherLine;
use doris_vat::domain::{DomainError, Submission, Vat, VatEvent, VatStatus, set_vat_period, settlement, status, submit};
use doris_vat::period::{VatPeriod, VatPeriodKind};
use std::collections::HashSet;

fn lines(list: &[VoucherLine]) -> Vec<(u16, i64, i64)> {
    list.iter().map(|l| (l.account.get(), l.debit, l.credit)).collect()
}

fn q3() -> VatPeriod {
    VatPeriod { start: d("2026-07-01"), end: d("2026-09-30") }
}

fn year26() -> FiscalYear {
    FiscalYear { start: d("2026-01-01"), end: d("2026-12-31") }
}

fn ver(n: u32) -> VoucherRef {
    VoucherRef { fiscal_year_start: d("2026-01-01"), number: n }
}

fn sales() -> Vec<AccountSaldo> {
    vec![a(2611, 10, -1_000_50), a(2640, 48, 400_30), a(3001, 5, -4_002_00)]
}

/// Submits `accounts` for Q3 on 2026-10-06 and records it with voucher `n`.
fn submitted(vat: &mut Vat, accounts: Vec<AccountSaldo>, n: u32) -> Vec<VoucherLine> {
    let print = fingerprint(q3().end, &accounts);
    let prepared = submit(vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), accounts, &print, &HashSet::new()).unwrap();
    let mut submission = prepared.submission;
    submission.voucher = Some(ver(n));
    vat.apply(&VatEvent::VatReturnSubmitted(submission));
    prepared.lines
}

#[test]
fn the_settlement_moves_the_vat_to_2650_and_the_ore_to_3740() {
    let accounts = sales();
    let s = settlement(&accounts, &boxes(&accounts), &[]);
    // 2611 debit 1 000,50; 2640 credit 400,30; 2650 credit box 49 = 600 kr;
    // 3740 takes the 0,20 left.
    assert_eq!(lines(&s.lines), [(2611, 1_000_50, 0), (2640, 0, 400_30), (2650, 0, 600_00), (3740, 0, 20)]);
    assert_eq!(s.settled, [(2611, -1_000_50), (2640, 400_30)]);
    assert_eq!(s.vat_due, 600_00);
}

#[test]
fn vat_to_get_back_is_a_debit_on_2650() {
    let accounts = [a(2611, 10, -100_00), a(2640, 48, 350_40)];
    let s = settlement(&accounts, &boxes(&accounts), &[]);
    assert_eq!(lines(&s.lines), [(2611, 100_00, 0), (2640, 0, 350_40), (2650, 250_00, 0), (3740, 40, 0)]);
}

#[test]
fn a_second_submission_settles_only_the_difference() {
    let mut vat = Vat::default();
    submitted(&mut vat, sales(), 7);
    // A late sale: 100 kr more output VAT.
    let more = vec![a(2611, 10, -1_100_50), a(2640, 48, 400_30), a(3001, 5, -4_402_00)];
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &more, &HashSet::new()), VatStatus::Changed);
    let lines_now = submitted(&mut vat, more, 9);
    assert_eq!(lines(&lines_now), [(2611, 100_00, 0), (2650, 0, 100_00)]);
}

#[test]
fn an_account_that_lost_its_vat_box_is_settled_back() {
    let mut vat = Vat::default();
    let with_reverse_charge = vec![a(2614, 30, -250_00), a(2645, 48, 250_00), a(4535, 21, 1_000_00)];
    submitted(&mut vat, with_reverse_charge, 7);
    // 2614 no longer has a box: it is no longer in the period's accounts.
    let now = vec![a(2645, 48, 250_00), a(4535, 21, 1_000_00)];
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &now, &HashSet::new()), VatStatus::Changed);
    let back = submitted(&mut vat, now, 9);
    // What was moved off 2614 goes back; 49 drops by 250 kr.
    assert_eq!(lines(&back), [(2614, 0, 250_00), (2650, 250_00, 0)]);
}

#[test]
fn a_corrected_settlement_counts_as_not_booked() {
    let mut vat = Vat::default();
    submitted(&mut vat, sales(), 7);
    let corrected = HashSet::from([ver(7)]);
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &sales(), &corrected), VatStatus::Changed);
    let print = fingerprint(q3().end, &sales());
    let again = submit(&vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), sales(), &print, &corrected).unwrap();
    assert_eq!(again.lines.len(), 4, "the whole settlement again");
}

#[test]
fn statuses_follow_the_date_and_the_latest_submission() {
    let mut vat = Vat::default();
    let none = HashSet::new();
    assert_eq!(status(&vat, q3(), d("2026-09-30"), &sales(), &none), VatStatus::InProgress);
    assert_eq!(status(&vat, q3(), d("2026-10-01"), &sales(), &none), VatStatus::ToSubmit);
    submitted(&mut vat, sales(), 7);
    assert_eq!(status(&vat, q3(), d("2026-10-06"), &sales(), &none), VatStatus::Submitted);
}

#[test]
fn submitting_is_refused_when_not_due_outdated_unchanged_or_not_registered() {
    let mut vat = Vat::default();
    let print = fingerprint(q3().end, &sales());
    let none = HashSet::new();
    let try_on = |vat: &Vat, kind, today: &str, print: &str| {
        submit(vat, q3(), kind, d(today), sales(), print, &none).map(|_| ())
    };
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-09-30", &print), Err(DomainError::VatPeriodNotEnded));
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-10-06", "stale"), Err(DomainError::VatReturnOutdated));
    assert_eq!(try_on(&vat, VatPeriodKind::NotRegistered, "2026-10-06", &print), Err(DomainError::VatNotRegistered));
    submitted(&mut vat, sales(), 7);
    assert_eq!(try_on(&vat, VatPeriodKind::Quarterly, "2026-10-06", &print), Err(DomainError::VatReturnUnchanged));
}

#[test]
fn nothing_to_settle_gives_no_lines() {
    let vat = Vat::default();
    let accounts = vec![a(3004, 42, -500_00)];
    let print = fingerprint(q3().end, &accounts);
    let prepared = submit(&vat, q3(), VatPeriodKind::Quarterly, d("2026-10-06"), accounts, &print, &HashSet::new()).unwrap();
    assert!(prepared.lines.is_empty());
    assert_eq!(prepared.submission.boxes.get(42), 500);
}

#[test]
fn the_period_kind_is_quarterly_until_set_and_locked_once_the_year_has_a_submission() {
    let mut vat = Vat::default();
    assert_eq!(vat.kind(d("2026-01-01")), VatPeriodKind::Quarterly);
    assert_eq!(set_vat_period(&vat, year26(), VatPeriodKind::Quarterly).unwrap(), []);
    let events = set_vat_period(&vat, year26(), VatPeriodKind::Monthly).unwrap();
    assert_eq!(events, [VatEvent::VatPeriodSet { fiscal_year_start: d("2026-01-01"), kind: VatPeriodKind::Monthly }]);
    vat.apply(&events[0]);
    assert_eq!(vat.kind(d("2026-01-01")), VatPeriodKind::Monthly);
    vat.apply(&VatEvent::VatReturnSubmitted(Submission {
        period_end: d("2026-01-31"),
        accounts: vec![],
        boxes: Default::default(),
        settled: vec![],
        settled_vat_due: 0,
        voucher: None,
    }));
    assert_eq!(set_vat_period(&vat, year26(), VatPeriodKind::Yearly), Err(DomainError::VatPeriodLocked));
    // Another year is not locked.
    let next = FiscalYear { start: d("2027-01-01"), end: d("2027-12-31") };
    assert!(set_vat_period(&vat, next, VatPeriodKind::Yearly).is_ok());
}

#[test]
fn events_round_trip_as_tagged_json() {
    let event = VatEvent::VatPeriodSet { fiscal_year_start: d("2026-01-01"), kind: VatPeriodKind::NotRegistered };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "VatPeriodSet");
    assert_eq!(json["kind"], "not_registered");
    assert_eq!(serde_json::from_value::<VatEvent>(json).unwrap(), event);
}
