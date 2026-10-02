use doris_ledger::domain::*;

fn n(number: u32) -> AccountNumber {
    AccountNumber::parse(number).unwrap()
}

fn name(raw: &str) -> AccountName {
    AccountName::parse(raw).unwrap()
}

fn seeded() -> Chart {
    Chart::from_events(&[seed_chart()])
}

#[test]
fn account_numbers_are_four_digits_from_1000_to_8999() {
    assert_eq!(n(1000).get(), 1000);
    assert_eq!(n(8999).get(), 8999);
    for bad in [0, 999, 9000, 19300] {
        assert_eq!(
            AccountNumber::parse(bad),
            Err(DomainError::InvalidAccountNumber),
            "{bad}"
        );
    }
}

#[test]
fn account_names_are_trimmed_and_1_to_100_characters() {
    assert_eq!(name("  Kassa ").as_str(), "Kassa");
    assert_eq!(name(&"å".repeat(100)).as_str().chars().count(), 100);
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(
            AccountName::parse(bad),
            Err(DomainError::InvalidAccountName)
        );
    }
}

#[test]
fn the_seed_is_a_bas_selection_with_every_account_active() {
    let chart = seeded();
    let count = chart.accounts().count();
    assert!((150..=250).contains(&count), "{count} accounts");
    let bank = chart.get(n(1930)).unwrap();
    assert_eq!(bank.name.as_str(), "Företagskonto/checkkonto/affärskonto");
    assert!(chart.accounts().all(|a| a.active));
    for number in [
        1510, 2440, 2611, 2641, 2650, 3001, 4010, 5010, 6570, 7510, 8999,
    ] {
        assert!(chart.get(n(number)).is_some(), "{number} missing");
    }
}

#[test]
fn an_account_is_added_once() {
    let chart = seeded();

    let events = add_account(&chart, n(1931), name("Sparkonto")).unwrap();

    assert_eq!(
        events,
        vec![ChartEvent::AccountAdded {
            number: n(1931),
            name: name("Sparkonto")
        }]
    );
    assert_eq!(
        add_account(&chart, n(1930), name("Bank")),
        Err(DomainError::AccountExists)
    );
}

#[test]
fn renaming_needs_an_existing_account_and_a_new_name() {
    let chart = seeded();

    assert_eq!(
        rename_account(&chart, n(1930), name("Bank")).unwrap(),
        vec![ChartEvent::AccountRenamed {
            number: n(1930),
            name: name("Bank")
        }]
    );
    assert_eq!(
        rename_account(
            &chart,
            n(1930),
            name("Företagskonto/checkkonto/affärskonto")
        )
        .unwrap(),
        vec![]
    );
    assert_eq!(
        rename_account(&chart, n(1999), name("Bank")),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn deactivating_and_reactivating_are_idempotent() {
    let mut chart = seeded();

    let off = set_account_active(&chart, n(1930), false).unwrap();
    assert_eq!(
        off,
        vec![ChartEvent::AccountDeactivated { number: n(1930) }]
    );
    chart.apply(&off[0]);
    assert!(!chart.get(n(1930)).unwrap().active);
    assert_eq!(set_account_active(&chart, n(1930), false).unwrap(), vec![]);

    let on = set_account_active(&chart, n(1930), true).unwrap();
    assert_eq!(on, vec![ChartEvent::AccountReactivated { number: n(1930) }]);
    chart.apply(&on[0]);
    assert!(chart.get(n(1930)).unwrap().active);
    assert_eq!(
        set_account_active(&chart, n(1999), true),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn chart_events_are_tagged_json_with_plain_numbers() {
    let json = serde_json::to_value(ChartEvent::AccountAdded {
        number: n(1931),
        name: name("Sparkonto"),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({"type": "AccountAdded", "number": 1931, "name": "Sparkonto"})
    );
}

use doris_company::domain::{FiscalYear, LegalForm};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn first_year() -> FiscalYear {
    FiscalYear::first(d("2025-01-01"), d("2025-12-31"), LegalForm::Aktiebolag).unwrap()
}

fn line(account: u32, debit: i64, credit: i64) -> VoucherLine {
    VoucherLine::new(account, debit, credit).unwrap()
}

fn sale(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Försäljning".into(),
        lines: vec![line(1930, ore, 0), line(3001, 0, ore)],
    }
}

/// Given these vouchers in the 2025 ledger, the decision on `cmd`.
fn record(
    given: &[LedgerEvent],
    chart: &Chart,
    cmd: RecordVoucher,
) -> Result<LedgerEvent, DomainError> {
    record_voucher(&Ledger::from_events(first_year(), given), chart, cmd)
}

fn number_of(event: &LedgerEvent) -> u32 {
    let LedgerEvent::VoucherRecorded { number, .. } = event;
    *number
}

#[test]
fn the_first_voucher_is_number_1_and_the_next_follows_the_last() {
    let chart = seeded();
    let first = record(&[], &chart, sale("2025-03-01", 10_000)).unwrap();
    assert_eq!(
        first,
        LedgerEvent::VoucherRecorded {
            number: 1,
            date: d("2025-03-01"),
            text: "Försäljning".into(),
            lines: vec![line(1930, 10_000, 0), line(3001, 0, 10_000)],
            corrects: None,
        }
    );

    let mut given = vec![first];
    for _ in 0..2 {
        let next = record(&given, &chart, sale("2025-03-02", 500)).unwrap();
        given.push(next);
    }
    assert_eq!(given.iter().map(number_of).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(
        number_of(&record(&given, &chart, sale("2025-01-01", 1)).unwrap()),
        4
    );
}

#[test]
fn voucher_text_is_trimmed_and_1_to_200_characters() {
    let chart = seeded();
    let mut cmd = sale("2025-03-01", 100);
    cmd.text = "  Hyra mars ".into();
    let LedgerEvent::VoucherRecorded { text, .. } = record(&[], &chart, cmd).unwrap();
    assert_eq!(text, "Hyra mars");
    for bad in ["", "  ", &"x".repeat(201)] {
        let mut cmd = sale("2025-03-01", 100);
        cmd.text = bad.into();
        assert_eq!(
            record(&[], &chart, cmd),
            Err(DomainError::InvalidVoucherText)
        );
    }
}

#[test]
fn a_voucher_has_2_to_100_lines() {
    let chart = seeded();
    let one = RecordVoucher {
        lines: vec![line(1930, 100, 0)],
        ..sale("2025-03-01", 100)
    };
    assert_eq!(
        record(&[], &chart, one),
        Err(DomainError::InvalidVoucherLines)
    );

    let mut many = vec![line(1930, 99, 0)];
    many.extend((0..99).map(|_| line(3001, 0, 1)));
    let hundred = RecordVoucher {
        lines: many.clone(),
        ..sale("2025-03-01", 1)
    };
    assert!(record(&[], &chart, hundred).is_ok());
    many.push(line(3001, 0, 1));
    let too_many = RecordVoucher {
        lines: many,
        ..sale("2025-03-01", 1)
    };
    assert_eq!(
        record(&[], &chart, too_many),
        Err(DomainError::InvalidVoucherLines)
    );
}

#[test]
fn each_line_has_exactly_one_positive_amount_within_the_limit() {
    let chart = seeded();
    for bad in [
        (0, 0),
        (100, 100),
        (-100, 0),
        (0, -100),
        (MAX_AMOUNT + 1, 0),
    ] {
        let cmd = RecordVoucher {
            lines: vec![line(1930, bad.0, bad.1), line(3001, 0, 100)],
            ..sale("2025-03-01", 100)
        };
        assert_eq!(
            record(&[], &chart, cmd),
            Err(DomainError::InvalidAmount),
            "{bad:?}"
        );
    }
    let max = RecordVoucher {
        lines: vec![line(1930, MAX_AMOUNT, 0), line(3001, 0, MAX_AMOUNT)],
        ..sale("2025-03-01", 1)
    };
    assert!(record(&[], &chart, max).is_ok());
}

#[test]
fn debit_must_equal_credit() {
    let cmd = RecordVoucher {
        lines: vec![line(1930, 10_000, 0), line(3001, 0, 9_999)],
        ..sale("2025-03-01", 1)
    };
    assert_eq!(
        record(&[], &seeded(), cmd),
        Err(DomainError::VoucherUnbalanced)
    );
}

#[test]
fn every_account_must_exist_and_be_active() {
    let mut chart = seeded();
    let unknown = RecordVoucher {
        lines: vec![line(1999, 100, 0), line(3001, 0, 100)],
        ..sale("2025-03-01", 1)
    };
    assert_eq!(
        record(&[], &chart, unknown),
        Err(DomainError::AccountNotFound)
    );
    assert_eq!(
        VoucherLine::new(19300, 100, 0),
        Err(DomainError::AccountNotFound)
    );

    chart.apply(&ChartEvent::AccountDeactivated { number: n(3001) });
    assert_eq!(
        record(&[], &chart, sale("2025-03-01", 100)),
        Err(DomainError::AccountInactive)
    );
}

#[test]
fn vouchers_on_fiscal_year_boundaries_land_in_the_right_year() {
    let first = first_year();
    let today = d("2026-06-15");
    assert_eq!(
        fiscal_year_for(first, d("2025-01-01"), today).unwrap(),
        first
    );
    assert_eq!(
        fiscal_year_for(first, d("2025-12-31"), today).unwrap(),
        first
    );
    assert_eq!(
        fiscal_year_for(first, d("2026-01-01"), today)
            .unwrap()
            .start,
        d("2026-01-01")
    );
    assert_eq!(
        fiscal_year_for(first, today, today).unwrap().end,
        d("2026-12-31")
    );
    assert_eq!(
        fiscal_year_for(first, d("2026-06-16"), today),
        Err(DomainError::VoucherDateInFuture)
    );
    assert_eq!(
        fiscal_year_for(first, d("2024-12-31"), today),
        Err(DomainError::VoucherDateBeforeFirstFiscalYear)
    );
}

#[test]
fn a_correction_reverses_every_line_and_points_at_the_original() {
    let chart = seeded();
    let original = record(&[], &chart, sale("2025-03-01", 10_000)).unwrap();
    let ledger = Ledger::from_events(first_year(), &[original]);

    let correction = correct_voucher(&ledger, 1, d("2025-03-05"), d("2026-01-10")).unwrap();

    assert_eq!(
        correction,
        LedgerEvent::VoucherRecorded {
            number: 2,
            date: d("2025-03-05"),
            text: "Rättelse av ver 1".into(),
            lines: vec![line(1930, 0, 10_000), line(3001, 10_000, 0)],
            corrects: Some(1),
        }
    );
    let after = Ledger::from_events(
        first_year(),
        &[
            record(&[], &chart, sale("2025-03-01", 10_000)).unwrap(),
            correction,
        ],
    );
    assert_eq!(after.voucher(1).unwrap().corrected_by, Some(2));
    assert_eq!(after.voucher(2).unwrap().corrects, Some(1));
}

#[test]
fn a_voucher_is_corrected_once_and_a_correction_never() {
    let chart = seeded();
    let original = record(&[], &chart, sale("2025-03-01", 100)).unwrap();
    let mut ledger = Ledger::from_events(first_year(), &[original]);
    let today = d("2025-06-01");
    ledger.apply(&correct_voucher(&ledger, 1, d("2025-03-02"), today).unwrap());

    assert_eq!(
        correct_voucher(&ledger, 1, d("2025-03-02"), today),
        Err(DomainError::AlreadyCorrected)
    );
    assert_eq!(
        correct_voucher(&ledger, 2, d("2025-03-02"), today),
        Err(DomainError::CannotCorrectCorrection)
    );
    assert_eq!(
        correct_voucher(&ledger, 3, d("2025-03-02"), today),
        Err(DomainError::VoucherNotFound)
    );
    assert_eq!(
        correct_voucher(&ledger, 0, d("2025-03-02"), today),
        Err(DomainError::VoucherNotFound)
    );
}

#[test]
fn a_correction_must_be_dated_inside_the_original_fiscal_year() {
    let chart = seeded();
    let ledger = Ledger::from_events(
        first_year(),
        &[record(&[], &chart, sale("2025-03-01", 100)).unwrap()],
    );
    let today = d("2026-02-01");

    assert!(correct_voucher(&ledger, 1, d("2025-12-31"), today).is_ok());
    assert_eq!(
        correct_voucher(&ledger, 1, d("2026-01-01"), today),
        Err(DomainError::CorrectionDateOutsideFiscalYear)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2024-12-31"), today),
        Err(DomainError::CorrectionDateOutsideFiscalYear)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2025-06-02"), d("2025-06-01")),
        Err(DomainError::VoucherDateInFuture)
    );
}

#[test]
fn a_correction_works_even_when_an_account_has_since_been_deactivated() {
    let mut chart = seeded();
    let ledger = Ledger::from_events(
        first_year(),
        &[record(&[], &chart, sale("2025-03-01", 100)).unwrap()],
    );
    chart.apply(&ChartEvent::AccountDeactivated { number: n(3001) });

    // correct_voucher never looks at the chart.
    assert!(correct_voucher(&ledger, 1, d("2025-03-02"), d("2025-06-01")).is_ok());
}

#[test]
fn voucher_events_are_readable_json() {
    let event = record(&[], &seeded(), sale("2025-03-01", 12_550)).unwrap();
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({
            "type": "VoucherRecorded",
            "number": 1,
            "date": "2025-03-01",
            "text": "Försäljning",
            "lines": [
                {"account": 1930, "debit": 12550, "credit": 0},
                {"account": 3001, "debit": 0, "credit": 12550}
            ],
            "corrects": null
        })
    );
}
