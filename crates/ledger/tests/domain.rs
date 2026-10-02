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
    match event {
        LedgerEvent::VoucherRecorded { number, .. } => *number,
        other => panic!("not a voucher: {other:?}"),
    }
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
    let LedgerEvent::VoucherRecorded { text, .. } = record(&[], &chart, cmd).unwrap() else {
        panic!("not a voucher");
    };
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

#[test]
fn the_running_balance_adds_debit_and_subtracts_credit_in_the_given_order() {
    let entries = running_balance(vec![
        (d("2026-01-05"), 1, "Försäljning".into(), 1000, 0),
        (d("2026-01-09"), 3, "Hyra".into(), 0, 1500),
        (d("2026-01-20"), 2, "Insättning".into(), 200, 0),
    ])
    .unwrap();

    assert_eq!(
        entries.iter().map(|e| e.balance).collect::<Vec<_>>(),
        [1000, -500, -300]
    );
    assert_eq!(
        entries[1],
        LedgerEntry {
            date: d("2026-01-09"),
            number: 3,
            text: "Hyra".into(),
            debit: 0,
            credit: 1500,
            balance: -500,
        }
    );
}

#[test]
fn no_lines_give_no_entries() {
    assert_eq!(running_balance(Vec::new()), Some(Vec::new()));
}

#[test]
fn an_overflowing_running_balance_is_none_not_a_panic() {
    let t = d("2026-01-01");
    assert_eq!(
        running_balance(vec![
            (t, 1, "a".into(), i64::MAX, 0),
            (t, 2, "b".into(), 1, 0)
        ]),
        None
    );
    assert_eq!(
        running_balance(vec![
            (t, 1, "a".into(), 0, i64::MAX),
            (t, 2, "b".into(), 0, 2)
        ]),
        None
    );
}

/// The 2025 ledger after `given`, closed.
fn closed_year(mut given: Vec<LedgerEvent>) -> Ledger {
    given.push(LedgerEvent::FiscalYearClosed {
        result_voucher: None,
    });
    Ledger::from_events(first_year(), &given)
}

#[test]
fn a_closed_year_takes_no_voucher_and_no_correction() {
    let chart = seeded();
    let booked = record(&[], &chart, sale("2025-03-01", 100)).unwrap();
    let ledger = closed_year(vec![booked]);

    assert!(ledger.is_closed());
    assert_eq!(
        record_voucher(&ledger, &chart, sale("2025-03-02", 100)),
        Err(DomainError::FiscalYearClosed)
    );
    assert_eq!(
        correct_voucher(&ledger, 1, d("2025-03-02"), d("2026-10-02")),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn a_reopened_year_takes_vouchers_again() {
    let chart = seeded();
    let ledger = Ledger::from_events(
        first_year(),
        &[
            LedgerEvent::FiscalYearClosed {
                result_voucher: None,
            },
            LedgerEvent::FiscalYearReopened {
                reason: "Glömd faktura".into(),
            },
        ],
    );

    assert!(!ledger.is_closed());
    assert!(record_voucher(&ledger, &chart, sale("2025-03-02", 100)).is_ok());
}

fn lines(raw: &[(u32, i64, i64)]) -> Vec<VoucherLine> {
    raw.iter()
        .map(|&(account, debit, credit)| line(account, debit, credit))
        .collect()
}

#[test]
fn balanced_opening_balances_on_balance_sheet_accounts_are_set() {
    let chart = seeded();
    let ib = lines(&[(1930, 10_000, 0), (2081, 0, 10_000)]);

    let event = set_opening_balances(&Ledger::new(first_year()), &chart, ib.clone()).unwrap();

    assert_eq!(event, LedgerEvent::OpeningBalancesSet { lines: ib.clone() });
    let ledger = Ledger::from_events(first_year(), &[event]);
    assert_eq!(ledger.opening_balances(), &ib[..]);
    // An empty list clears them.
    assert_eq!(
        set_opening_balances(&ledger, &chart, Vec::new()),
        Ok(LedgerEvent::OpeningBalancesSet { lines: Vec::new() })
    );
}

#[test]
fn an_inactive_account_may_carry_an_opening_balance() {
    let mut chart = seeded();
    for event in set_account_active(&chart, n(1910), false).unwrap() {
        chart.apply(&event);
    }

    let ib = lines(&[(1910, 500, 0), (2081, 0, 500)]);

    assert!(set_opening_balances(&Ledger::new(first_year()), &chart, ib).is_ok());
}

#[test]
fn invalid_opening_balances_are_refused() {
    let chart = seeded();
    let open = Ledger::new(first_year());
    let too_many: Vec<VoucherLine> = (0..=MAX_OPENING_BALANCE_LINES)
        .map(|_| line(1930, 1, 0))
        .collect();
    for (ib, expected) in [
        (too_many, DomainError::InvalidVoucherLines),
        (
            lines(&[(1930, 0, 0), (2081, 0, 0)]),
            DomainError::InvalidAmount,
        ),
        (
            lines(&[(1930, 5, 5), (2081, 0, 0)]),
            DomainError::InvalidAmount,
        ),
        (
            lines(&[(1930, 100, 0), (3001, 0, 100)]),
            DomainError::NotBalanceSheetAccount,
        ),
        (
            lines(&[(1999, 100, 0), (2081, 0, 100)]),
            DomainError::AccountNotFound,
        ),
        (
            lines(&[(1930, 100, 0), (1930, 0, 100)]),
            DomainError::DuplicateAccount,
        ),
        (
            lines(&[(1930, 100, 0), (2081, 0, 99)]),
            DomainError::OpeningBalancesUnbalanced,
        ),
    ] {
        assert_eq!(
            set_opening_balances(&open, &chart, ib),
            Err(expected),
            "{expected:?}"
        );
    }
    assert_eq!(
        set_opening_balances(
            &closed_year(Vec::new()),
            &chart,
            lines(&[(1930, 100, 0), (2081, 0, 100)])
        ),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn closing_events_are_readable_json() {
    let as_json = |event: LedgerEvent| serde_json::to_value(event).unwrap();

    assert_eq!(
        as_json(LedgerEvent::OpeningBalancesSet {
            lines: vec![line(1930, 100, 0)]
        }),
        serde_json::json!({
            "type": "OpeningBalancesSet",
            "lines": [{"account": 1930, "debit": 100, "credit": 0}]
        })
    );
    assert_eq!(
        as_json(LedgerEvent::FiscalYearClosed {
            result_voucher: Some(7)
        }),
        serde_json::json!({"type": "FiscalYearClosed", "result_voucher": 7})
    );
    assert_eq!(
        as_json(LedgerEvent::FiscalYearReopened {
            reason: "Glömd faktura".into()
        }),
        serde_json::json!({"type": "FiscalYearReopened", "reason": "Glömd faktura"})
    );
}

const AFTER_2025: &str = "2026-10-02";

fn rent(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Hyra".into(),
        lines: vec![line(5010, ore, 0), line(1930, 0, ore)],
    }
}

/// The 2025 events after booking each command in turn.
fn year_with(cmds: Vec<RecordVoucher>) -> Vec<LedgerEvent> {
    let chart = seeded();
    let mut events = Vec::new();
    for cmd in cmds {
        let event = record(&events, &chart, cmd).unwrap();
        events.push(event);
    }
    events
}

fn close_2025(given: &[LedgerEvent], form: LegalForm) -> Vec<LedgerEvent> {
    close_fiscal_year(
        &Ledger::from_events(first_year(), given),
        None,
        form,
        d(AFTER_2025),
    )
    .unwrap()
}

#[test]
fn closing_a_profitable_year_books_8999_against_2099_on_its_last_day() {
    let given = year_with(vec![sale("2025-03-01", 1_000), rent("2025-04-01", 300)]);
    assert_eq!(
        result_of(&Ledger::from_events(first_year(), &given)),
        Some(-700)
    );

    let events = close_2025(&given, LegalForm::Aktiebolag);

    assert_eq!(
        events,
        vec![
            LedgerEvent::VoucherRecorded {
                number: 3,
                date: d("2025-12-31"),
                text: "Årets resultat".into(),
                lines: vec![line(8999, 700, 0), line(2099, 0, 700)],
                corrects: None,
            },
            LedgerEvent::FiscalYearClosed {
                result_voucher: Some(3)
            },
        ]
    );
}

#[test]
fn a_loss_is_booked_the_other_way_and_owners_taxed_personally_use_2019() {
    let given = year_with(vec![rent("2025-04-01", 300)]);
    for (form, equity) in [
        (LegalForm::Aktiebolag, 2099),
        (LegalForm::EkonomiskForening, 2099),
        (LegalForm::EnskildFirma, 2019),
        (LegalForm::Handelsbolag, 2019),
        (LegalForm::Kommanditbolag, 2019),
    ] {
        let events = close_2025(&given, form);
        let LedgerEvent::VoucherRecorded { lines, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(
            lines,
            &vec![line(8999, 0, 300), line(equity, 300, 0)],
            "{form:?}"
        );
    }
}

#[test]
fn a_year_whose_result_is_already_on_equity_closes_without_a_voucher() {
    let booked_by_hand = year_with(vec![
        sale("2025-03-01", 1_000),
        RecordVoucher {
            date: d("2025-12-31"),
            text: "Årets resultat".into(),
            lines: vec![line(8999, 1_000, 0), line(2099, 0, 1_000)],
        },
    ]);
    let closed = vec![LedgerEvent::FiscalYearClosed {
        result_voucher: None,
    }];

    assert_eq!(close_2025(&booked_by_hand, LegalForm::Aktiebolag), closed);
    assert_eq!(close_2025(&[], LegalForm::Aktiebolag), closed);
}

#[test]
fn a_year_closes_once_after_it_has_ended_and_after_the_year_before() {
    let open = Ledger::new(first_year());
    let close = |ledger: &Ledger, previous: Option<bool>, today: &str| {
        close_fiscal_year(ledger, previous, LegalForm::Aktiebolag, d(today))
    };

    assert_eq!(
        close(&open, None, "2025-12-31"),
        Err(DomainError::FiscalYearNotEnded)
    );
    assert!(close(&open, None, "2026-01-01").is_ok());
    assert_eq!(
        close(&open, Some(false), AFTER_2025),
        Err(DomainError::PreviousFiscalYearOpen)
    );
    assert!(close(&open, Some(true), AFTER_2025).is_ok());
    assert_eq!(
        close(&closed_year(Vec::new()), Some(true), AFTER_2025),
        Err(DomainError::FiscalYearClosed)
    );
}

#[test]
fn an_overflowing_result_is_an_error_not_a_wrong_voucher() {
    let huge = LedgerEvent::VoucherRecorded {
        number: 1,
        date: d("2025-03-01"),
        text: "x".into(),
        lines: vec![line(3001, 0, i64::MAX), line(3002, 0, i64::MAX)],
        corrects: None,
    };
    let ledger = Ledger::from_events(first_year(), &[huge]);

    assert_eq!(result_of(&ledger), None);
    assert_eq!(
        close_fiscal_year(&ledger, None, LegalForm::Aktiebolag, d(AFTER_2025)),
        Err(DomainError::Overflow)
    );
}

#[test]
fn reopening_comes_first_and_then_reverses_the_result_voucher() {
    let mut given = year_with(vec![sale("2025-03-01", 1_000)]);
    given.extend(close_2025(&given, LegalForm::Aktiebolag));
    let ledger = Ledger::from_events(first_year(), &given);

    let events = reopen_fiscal_year(&ledger, false, "  Glömd faktura ").unwrap();

    assert_eq!(
        events,
        vec![
            LedgerEvent::FiscalYearReopened {
                reason: "Glömd faktura".into()
            },
            LedgerEvent::VoucherRecorded {
                number: 3,
                date: d("2025-12-31"),
                text: "Rättelse av ver 2".into(),
                lines: vec![line(8999, 0, 1_000), line(2099, 1_000, 0)],
                corrects: Some(2),
            },
        ]
    );
}

#[test]
fn reopening_without_a_result_voucher_reverses_nothing() {
    assert_eq!(
        reopen_fiscal_year(&closed_year(Vec::new()), false, "Fel"),
        Ok(vec![LedgerEvent::FiscalYearReopened {
            reason: "Fel".into()
        }])
    );
}

#[test]
fn only_a_closed_year_without_a_closed_successor_reopens_and_only_with_a_reason() {
    assert_eq!(
        reopen_fiscal_year(&Ledger::new(first_year()), false, "Fel"),
        Err(DomainError::FiscalYearOpen)
    );
    assert_eq!(
        reopen_fiscal_year(&closed_year(Vec::new()), true, "Fel"),
        Err(DomainError::LaterFiscalYearClosed)
    );
    for bad in ["", "   ", &"å".repeat(201)] {
        assert_eq!(
            reopen_fiscal_year(&closed_year(Vec::new()), false, bad),
            Err(DomainError::InvalidReason),
            "{bad:?}"
        );
    }
    assert!(reopen_fiscal_year(&closed_year(Vec::new()), false, &"å".repeat(200)).is_ok());
}

#[test]
fn closing_again_after_a_reopen_books_the_new_result_without_gaps() {
    let chart = seeded();
    let mut given = year_with(vec![sale("2025-03-01", 1_000)]);
    given.extend(close_2025(&given, LegalForm::Aktiebolag)); // ver 2
    given.extend(
        reopen_fiscal_year(
            &Ledger::from_events(first_year(), &given),
            false,
            "Glömd hyra",
        )
        .unwrap(),
    ); // ver 3 reverses ver 2
    let forgotten = record(&given, &chart, rent("2025-12-15", 400)).unwrap(); // ver 4
    given.push(forgotten);
    given.extend(close_2025(&given, LegalForm::Aktiebolag)); // ver 5

    let ledger = Ledger::from_events(first_year(), &given);
    assert!(ledger.is_closed());
    assert_eq!(
        ledger
            .vouchers()
            .iter()
            .map(|v| v.number)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(
        ledger.voucher(5).unwrap().lines,
        vec![line(8999, 600, 0), line(2099, 0, 600)]
    );
    assert_eq!(result_of(&ledger), Some(0));
}
