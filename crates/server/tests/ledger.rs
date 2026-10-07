mod common;

use common::{Ledger, TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as pb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company with the given first räkenskapsår.
async fn company_starting(server: &TestServer, session: &str, start: &str, end: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: start.into(),
                fiscal_year_end: end.into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
}

/// Anna's company, first räkenskapsår 2026.
async fn company(server: &TestServer, session: &str) -> String {
    company_starting(server, session, "2026-01-01", "2026-12-31").await
}

fn sale(company_id: &str, ore: i64) -> pb::RecordVoucherRequest {
    pb::RecordVoucherRequest {
        company_id: company_id.into(),
        date: "2026-01-15".into(),
        text: "Försäljning".into(),
        lines: vec![
            pb::VoucherLine {
                account: 1930,
                debit: ore,
                credit: 0,
            },
            pb::VoucherLine {
                account: 3001,
                debit: 0,
                credit: ore,
            },
        ],
        attachments: vec![],
        dry_run: false,
    }
}

#[tokio::test]
async fn a_member_keeps_the_chart_and_books_and_corrects_vouchers() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();

    let accounts = api
        .list_accounts(authed(
            pb::ListAccountsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .accounts;
    assert!(accounts.iter().any(|a| a.number == 1930 && a.active));
    api.add_account(authed(
        pb::AddAccountRequest {
            company_id: id.clone(),
            number: 1931,
            name: "Sparkonto".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    api.rename_account(authed(
        pb::RenameAccountRequest {
            company_id: id.clone(),
            number: 1931,
            name: "Sparkonto SEB".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    api.set_account_active(authed(
        pb::SetAccountActiveRequest {
            company_id: id.clone(),
            number: 1910,
            active: false,
        },
        &anna,
    ))
    .await
    .unwrap();
    let accounts = api
        .list_accounts(authed(
            pb::ListAccountsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .accounts;
    assert!(accounts.contains(&pb::Account {
        number: 1931,
        name: "Sparkonto SEB".into(),
        active: true,
        vat_box: 0,
    }));
    assert!(accounts.contains(&pb::Account {
        number: 1910,
        name: "Kassa".into(),
        active: false,
        vat_box: 0,
    }));

    let years = api
        .list_fiscal_years(authed(
            pb::ListFiscalYearsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .fiscal_years;
    assert_eq!(
        years.last().unwrap(),
        &pb::FiscalYear {
            start: "2026-01-01".into(),
            end: "2026-12-31".into(),
            closed: false,
        }
    );

    let booked = api
        .record_voucher(authed(sale(&id, 12_500), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        booked,
        pb::RecordVoucherResponse {
            fiscal_year_start: "2026-01-01".into(),
            number: 1,
            ..Default::default()
        }
    );
    let corrected = api
        .correct_voucher(authed(
            pb::CorrectVoucherRequest {
                company_id: id.clone(),
                fiscal_year_start: "2026-01-01".into(),
                number: 1,
                date: "2026-01-16".into(),
                dry_run: false,
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(corrected.number, 2);

    let vouchers = api
        .list_vouchers(authed(
            pb::ListVouchersRequest {
                company_id: id,
                fiscal_year_start: "2026-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers;
    assert_eq!(
        vouchers[0],
        pb::Voucher {
            number: 1,
            date: "2026-01-15".into(),
            text: "Försäljning".into(),
            lines: sale("", 12_500).lines,
            corrects: 0,
            corrected_by: 2,
            attachments: vec![],
            recorded_at: vouchers[0].recorded_at.clone(),
            recorded_by_name: "Anna".into(),
        }
    );
    // RFC 3339 in UTC, and nothing of the email.
    assert!(
        vouchers[0].recorded_at.ends_with('Z'),
        "{}",
        vouchers[0].recorded_at
    );
    assert!(!format!("{:?}", vouchers[0]).contains('@'));
    assert_eq!(
        (vouchers[1].corrects, vouchers[1].text.as_str()),
        (1, "Rättelse av ver 1")
    );
}

#[tokio::test]
async fn ledger_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.set_account_active(authed(
        pb::SetAccountActiveRequest {
            company_id: id.clone(),
            number: 1910,
            active: false,
        },
        &anna,
    ))
    .await
    .unwrap();
    api.record_voucher(authed(sale(&id, 100), &anna))
        .await
        .unwrap();
    api.correct_voucher(authed(
        pb::CorrectVoucherRequest {
            company_id: id.clone(),
            fiscal_year_start: "2026-01-01".into(),
            number: 1,
            date: "2026-01-15".into(),
            dry_run: false,
        },
        &anna,
    ))
    .await
    .unwrap();

    let mut unbalanced = sale(&id, 100);
    unbalanced.lines[1].credit = 99;
    let mut inactive = sale(&id, 100);
    inactive.lines[0].account = 1910;
    let mut bad_date = sale(&id, 100);
    bad_date.date = "15/1".into();
    let mut future = sale(&id, 100);
    future.date = "2999-01-01".into();
    let mut before = sale(&id, 100);
    before.date = "2025-12-31".into();
    let mut no_text = sale(&id, 100);
    no_text.text = " ".into();
    let mut one_line = sale(&id, 100);
    one_line.lines.pop();
    let mut negative = sale(&id, 100);
    negative.lines[0].debit = -100;
    let mut unknown = sale(&id, 100);
    unknown.lines[0].account = 1999;

    for (request, expected) in [
        (unbalanced, (Code::InvalidArgument, "voucher_unbalanced")),
        (inactive, (Code::FailedPrecondition, "account_inactive")),
        (bad_date, (Code::InvalidArgument, "invalid_date")),
        (future, (Code::InvalidArgument, "voucher_date_in_future")),
        (
            before,
            (
                Code::InvalidArgument,
                "voucher_date_before_first_fiscal_year",
            ),
        ),
        (no_text, (Code::InvalidArgument, "invalid_voucher_text")),
        (one_line, (Code::InvalidArgument, "invalid_voucher_lines")),
        (negative, (Code::InvalidArgument, "invalid_amount")),
        (unknown, (Code::NotFound, "account_not_found")),
    ] {
        let err = api
            .record_voucher(authed(request, &anna))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (expected.0, expected.1.to_owned()));
    }

    let correct = |number: u32, date: &str| pb::CorrectVoucherRequest {
        company_id: id.clone(),
        fiscal_year_start: "2026-01-01".into(),
        number,
        date: date.into(),
        dry_run: false,
    };
    for (request, expected) in [
        (
            correct(1, "2026-01-15"),
            (Code::FailedPrecondition, "already_corrected"),
        ),
        (
            correct(2, "2026-01-15"),
            (Code::FailedPrecondition, "cannot_correct_correction"),
        ),
        (
            correct(9, "2026-01-15"),
            (Code::NotFound, "voucher_not_found"),
        ),
    ] {
        let err = api
            .correct_voucher(authed(request, &anna))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (expected.0, expected.1.to_owned()));
    }

    for (result, expected) in [
        (
            api.add_account(authed(
                pb::AddAccountRequest {
                    company_id: id.clone(),
                    number: 1930,
                    name: "X".into(),
                },
                &anna,
            ))
            .await
            .map(drop),
            (Code::AlreadyExists, "account_exists"),
        ),
        (
            api.add_account(authed(
                pb::AddAccountRequest {
                    company_id: id.clone(),
                    number: 99,
                    name: "X".into(),
                },
                &anna,
            ))
            .await
            .map(drop),
            (Code::InvalidArgument, "invalid_account_number"),
        ),
        (
            api.add_account(authed(
                pb::AddAccountRequest {
                    company_id: id.clone(),
                    number: 1931,
                    name: "".into(),
                },
                &anna,
            ))
            .await
            .map(drop),
            (Code::InvalidArgument, "invalid_account_name"),
        ),
        (
            api.rename_account(authed(
                pb::RenameAccountRequest {
                    company_id: id.clone(),
                    number: 1999,
                    name: "X".into(),
                },
                &anna,
            ))
            .await
            .map(drop),
            (Code::NotFound, "account_not_found"),
        ),
    ] {
        assert_eq!(
            code_of(result.unwrap_err()),
            (expected.0, expected.1.to_owned())
        );
    }
}

#[tokio::test]
async fn others_get_company_not_found_and_strangers_not_signed_in() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    for company_id in [id.clone(), "not-a-uuid".into()] {
        let err = api
            .list_accounts(authed(
                pb::ListAccountsRequest {
                    company_id: company_id.clone(),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), not_found);
        let err = api
            .record_voucher(authed(sale(&company_id, 100), &bo))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), not_found);
    }

    let unauthenticated = (Code::Unauthenticated, "not_signed_in".to_owned());
    let results = [
        api.list_accounts(pb::ListAccountsRequest {
            company_id: id.clone(),
        })
        .await
        .map(drop),
        api.add_account(pb::AddAccountRequest {
            company_id: id.clone(),
            number: 1931,
            name: "X".into(),
        })
        .await
        .map(drop),
        api.rename_account(pb::RenameAccountRequest {
            company_id: id.clone(),
            number: 1930,
            name: "X".into(),
        })
        .await
        .map(drop),
        api.set_account_active(pb::SetAccountActiveRequest {
            company_id: id.clone(),
            number: 1930,
            active: false,
        })
        .await
        .map(drop),
        api.list_fiscal_years(pb::ListFiscalYearsRequest {
            company_id: id.clone(),
        })
        .await
        .map(drop),
        api.record_voucher(sale(&id, 100)).await.map(drop),
        api.correct_voucher(pb::CorrectVoucherRequest {
            company_id: id.clone(),
            fiscal_year_start: "2026-01-01".into(),
            number: 1,
            date: "2026-01-15".into(),
            dry_run: false,
        })
        .await
        .map(drop),
        api.list_vouchers(pb::ListVouchersRequest {
            company_id: id,
            fiscal_year_start: "2026-01-01".into(),
        })
        .await
        .map(drop),
    ];
    for result in results {
        assert_eq!(code_of(result.unwrap_err()), unauthenticated);
    }
}

fn trial_balance_of(company_id: &str, fiscal_year_start: &str) -> pb::GetTrialBalanceRequest {
    pb::GetTrialBalanceRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

fn ledger_of(
    company_id: &str,
    fiscal_year_start: &str,
    account: u32,
) -> pb::GetAccountLedgerRequest {
    pb::GetAccountLedgerRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
        account,
    }
}

#[tokio::test]
async fn the_trial_balance_and_an_accounts_ledger_follow_the_vouchers() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    for ore in [125_000, 5_000] {
        api.record_voucher(authed(sale(&id, ore), &anna))
            .await
            .unwrap();
    }

    let rows = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner()
        .rows;
    let ledger = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &anna))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        rows,
        vec![
            pb::TrialBalanceRow {
                account: 1930,
                name: "Företagskonto/checkkonto/affärskonto".into(),
                debit: 130_000,
                credit: 0,
                opening: 0,
            },
            pb::TrialBalanceRow {
                account: 3001,
                name: "Försäljning inom Sverige, 25 % moms".into(),
                debit: 0,
                credit: 130_000,
                opening: 0,
            },
        ]
    );
    assert_eq!(ledger.opening, 0);
    assert_eq!(
        ledger.entries,
        vec![
            pb::LedgerEntry {
                date: "2026-01-15".into(),
                number: 1,
                text: "Försäljning".into(),
                debit: 125_000,
                credit: 0,
                balance: 125_000,
            },
            pb::LedgerEntry {
                date: "2026-01-15".into(),
                number: 2,
                text: "Försäljning".into(),
                debit: 5_000,
                credit: 0,
                balance: 130_000,
            },
        ]
    );
}

#[tokio::test]
async fn the_reports_refuse_bad_input_and_non_members() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let invalid_date = (Code::InvalidArgument, "invalid_date".to_owned());
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    let err = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-13-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), invalid_date);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "nonsense", 1930), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), invalid_date);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 99), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_account_number".to_owned())
    );

    let err = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), not_found);
    let err = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &bo))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), not_found);
    let err = api
        .get_trial_balance(trial_balance_of(&id, "2026-01-01"))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".to_owned())
    );
}

fn line(account: u32, debit: i64, credit: i64) -> pb::VoucherLine {
    pb::VoucherLine {
        account,
        debit,
        credit,
    }
}

fn close_of(company_id: &str, fiscal_year_start: &str) -> pb::CloseFiscalYearRequest {
    pb::CloseFiscalYearRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

fn reopen_of(
    company_id: &str,
    fiscal_year_start: &str,
    reason: &str,
) -> pb::ReopenFiscalYearRequest {
    pb::ReopenFiscalYearRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
        reason: reason.into(),
    }
}

#[tokio::test]
async fn opening_balances_and_a_closed_year_carry_into_the_next() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    // 2025 has ended by the time this runs.
    let id = company_starting(&server, &anna, "2025-01-01", "2025-12-31").await;
    let mut api = server.ledger();
    let ib = vec![line(1930, 10_000, 0), line(2081, 0, 10_000)];

    api.set_opening_balances(authed(
        pb::SetOpeningBalancesRequest {
            company_id: id.clone(),
            lines: ib.clone(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let saved = api
        .get_opening_balances(authed(
            pb::GetOpeningBalancesRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .lines;
    assert_eq!(saved, ib);
    let mut sale_2025 = sale(&id, 1_000);
    sale_2025.date = "2025-03-01".into();
    api.record_voucher(authed(sale_2025.clone(), &anna))
        .await
        .unwrap();

    let closed = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(closed.result_voucher, 2);

    let years = api
        .list_fiscal_years(authed(
            pb::ListFiscalYearsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .fiscal_years;
    let oldest = years.last().unwrap();
    assert_eq!((oldest.start.as_str(), oldest.closed), ("2025-01-01", true));
    assert!(!years[0].closed);

    let err = api
        .record_voucher(authed(sale_2025, &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "fiscal_year_closed".to_owned())
    );

    // 2026 opens with the balance sheet and the result on 2099.
    let rows = api
        .get_trial_balance(authed(trial_balance_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner()
        .rows;
    assert_eq!(
        rows.iter()
            .map(|r| (r.account, r.opening))
            .collect::<Vec<_>>(),
        vec![(1930, 11_000), (2081, -10_000), (2099, -1_000)]
    );
    let bank = api
        .get_account_ledger(authed(ledger_of(&id, "2026-01-01", 1930), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(bank.opening, 11_000);

    let err = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "company_not_found".to_owned())
    );
    api.reopen_fiscal_year(authed(reopen_of(&id, "2025-01-01", "Glömd faktura"), &anna))
        .await
        .unwrap();
}

#[tokio::test]
async fn closing_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    // 2024 and 2025 have both ended by the time this runs.
    let id = company_starting(&server, &anna, "2024-01-01", "2024-12-31").await;
    let mut api = server.ledger();
    let set = |lines: Vec<pb::VoucherLine>| {
        authed(
            pb::SetOpeningBalancesRequest {
                company_id: id.clone(),
                lines,
            },
            &anna,
        )
    };
    let expect = |err: tonic::Status, code: Code, message: &str| {
        assert_eq!(code_of(err), (code, message.to_owned()));
    };

    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(2081, 0, 99)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "opening_balances_unbalanced");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(3001, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "not_balance_sheet_account");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(1930, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "duplicate_account");

    let err = api
        .close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "previous_fiscal_year_open");
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2024-01-01", "Fel"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_open");
    let err = api
        .close_fiscal_year(authed(close_of(&id, "2024-02-01"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::NotFound, "fiscal_year_not_found");
    let err = api
        .close_fiscal_year(authed(close_of(&id, "nonsense"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "invalid_date");

    api.close_fiscal_year(authed(close_of(&id, "2024-01-01"), &anna))
        .await
        .unwrap();
    api.close_fiscal_year(authed(close_of(&id, "2025-01-01"), &anna))
        .await
        .unwrap();
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2025-01-01", "  "), &anna))
        .await
        .unwrap_err();
    expect(err, Code::InvalidArgument, "invalid_reason");
    let err = api
        .reopen_fiscal_year(authed(reopen_of(&id, "2024-01-01", "Fel"), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "later_fiscal_year_closed");
    let err = api
        .set_opening_balances(set(vec![line(1930, 100, 0), line(2081, 0, 100)]))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_closed");

    // The newest listed year contains today, so it has never ended. Every
    // year between 2025 and it must be closed first, oldest first.
    let years = api
        .list_fiscal_years(authed(
            pb::ListFiscalYearsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .fiscal_years;
    let (current, ended) = years.split_first().unwrap();
    for year in ended
        .iter()
        .rev()
        .filter(|y| y.start.as_str() > "2025-01-01")
    {
        api.close_fiscal_year(authed(close_of(&id, &year.start), &anna))
            .await
            .unwrap();
    }
    let err = api
        .close_fiscal_year(authed(close_of(&id, &current.start), &anna))
        .await
        .unwrap_err();
    expect(err, Code::FailedPrecondition, "fiscal_year_not_ended");
}

const MIB: usize = 1 << 20;

/// A PDF of exactly `size` bytes.
fn pdf(size: usize) -> Vec<u8> {
    let mut data = b"%PDF-1.7\n".to_vec();
    data.resize(size, b'x');
    data
}

fn upload(name: &str, data: Vec<u8>) -> pb::NewAttachment {
    pb::NewAttachment {
        file_name: name.into(),
        data,
    }
}

fn with_files(company_id: &str, files: Vec<pb::NewAttachment>) -> pb::RecordVoucherRequest {
    pb::RecordVoucherRequest {
        attachments: files,
        ..sale(company_id, 100)
    }
}

async fn refusal(
    api: &mut Ledger,
    session: &str,
    request: pb::RecordVoucherRequest,
) -> (Code, String) {
    code_of(
        api.record_voucher(authed(request, session))
            .await
            .unwrap_err(),
    )
}

#[tokio::test]
async fn underlag_go_up_with_a_voucher_and_come_back_byte_for_byte() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let big = pdf(10 * MIB);

    let booked = api
        .record_voucher(authed(
            with_files(&id, vec![upload("kvitto.pdf", big.clone())]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let listed = api
        .list_vouchers(authed(
            pb::ListVouchersRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers[0]
        .attachments
        .clone();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].file_name.as_str(),
            listed[0].content_type.as_str(),
            listed[0].size
        ),
        ("kvitto.pdf", "application/pdf", (10 * MIB) as u64)
    );
    assert_eq!(listed[0].id.len(), 64);

    let got = api
        .get_attachment(authed(
            pb::GetAttachmentRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start.clone(),
                number: booked.number,
                id: listed[0].id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.attachment.as_ref(), Some(&listed[0]));
    assert!(got.data == big, "the bytes differ");

    let added = api
        .add_attachment(authed(
            pb::AddAttachmentRequest {
                company_id: id.clone(),
                fiscal_year_start: booked.fiscal_year_start,
                number: booked.number,
                attachment: Some(upload("foto.png", b"\x89PNG\r\n\x1a\nbild".to_vec())),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .attachment
        .unwrap();
    assert_eq!(
        (added.file_name.as_str(), added.content_type.as_str()),
        ("foto.png", "image/png")
    );

    // Two files of 10 MiB fit in one request.
    let mut other = pdf(10 * MIB);
    other[20] = b'y';
    api.record_voucher(authed(
        with_files(
            &id,
            vec![upload("a.pdf", pdf(10 * MIB)), upload("b.pdf", other)],
        ),
        &anna,
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn attachment_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let invalid = |code: &str| (Code::InvalidArgument, code.to_owned());
    let mut other = pdf(10 * MIB);
    other[20] = b'y';

    assert_eq!(
        refusal(
            &mut api,
            &anna,
            with_files(&id, vec![upload("kvitto.pdf", pdf(10 * MIB + 1))])
        )
        .await,
        invalid("attachment_too_large")
    );
    // 20 MiB + 10 bytes: over the per-request total, under the 21 MiB message limit.
    assert_eq!(
        refusal(
            &mut api,
            &anna,
            with_files(
                &id,
                vec![
                    upload("a.pdf", pdf(10 * MIB)),
                    upload("b.pdf", other),
                    upload("c.pdf", pdf(10))
                ]
            )
        )
        .await,
        invalid("attachment_too_large")
    );
    assert_eq!(
        refusal(
            &mut api,
            &anna,
            with_files(&id, vec![upload("bild.gif", b"GIF89a".to_vec())])
        )
        .await,
        invalid("unsupported_attachment_type")
    );
    assert_eq!(
        refusal(&mut api, &anna, with_files(&id, vec![upload(" ", pdf(10))])).await,
        invalid("invalid_attachment_name")
    );
    assert_eq!(
        refusal(
            &mut api,
            &anna,
            with_files(&id, vec![upload("tom.pdf", vec![])])
        )
        .await,
        invalid("empty_attachment")
    );

    let booked = api
        .record_voucher(authed(
            with_files(&id, vec![upload("kvitto.pdf", pdf(10))]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let add = |number: u32| pb::AddAttachmentRequest {
        company_id: id.clone(),
        fiscal_year_start: booked.fiscal_year_start.clone(),
        number,
        attachment: Some(upload("kopia.pdf", pdf(10))),
    };
    assert_eq!(
        code_of(
            api.add_attachment(authed(add(booked.number), &anna))
                .await
                .unwrap_err()
        ),
        invalid("duplicate_attachment")
    );
    assert_eq!(
        code_of(
            api.add_attachment(authed(add(99), &anna))
                .await
                .unwrap_err()
        ),
        (Code::NotFound, "voucher_not_found".into())
    );
    let get = pb::GetAttachmentRequest {
        company_id: id.clone(),
        fiscal_year_start: booked.fiscal_year_start.clone(),
        number: booked.number,
        id: "0".repeat(64),
    };
    assert_eq!(
        code_of(
            api.get_attachment(authed(get.clone(), &anna))
                .await
                .unwrap_err()
        ),
        (Code::NotFound, "attachment_not_found".into())
    );
    let bertil = server.invite(&anna, &mut annas, "bertil@example.se").await;
    assert_eq!(
        code_of(api.get_attachment(authed(get, &bertil)).await.unwrap_err()),
        (Code::NotFound, "company_not_found".into())
    );
}

#[tokio::test]
async fn a_huge_frame_without_a_session_is_refused_before_its_body_is_read() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = TestServer::start().await;
    // The frame header claims 21 MiB, but only the header itself is sent.
    let claimed: u32 = 21 << 20;
    let mut header = vec![0u8];
    header.extend_from_slice(&claimed.to_be_bytes());
    let mut stream = tokio::net::TcpStream::connect(server.base.trim_start_matches("http://"))
        .await
        .unwrap();
    let request = format!(
        "POST /doris.ledger.v1.LedgerService/RecordVoucher HTTP/1.1\r\n\
         host: localhost\r\n\
         content-type: application/grpc-web+proto\r\n\
         x-grpc-web: 1\r\n\
         content-length: {}\r\n\r\n",
        claimed as usize + header.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(&header).await.unwrap();

    let mut response = Vec::new();
    let mut buf = [0u8; 1024];
    let head = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !response.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "connection closed without an answer");
            response.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&response).to_lowercase()
    })
    .await
    .expect("the server waited for the body instead of answering");

    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(head.contains("grpc-status: 16"), "{head}");
    assert!(head.contains("grpc-message: not_signed_in"), "{head}");
    assert!(
        head.contains("content-type: application/grpc-web"),
        "{head}"
    );
}

fn statements_of(company_id: &str, fiscal_year_start: &str) -> pb::GetFinancialStatementsRequest {
    pb::GetFinancialStatementsRequest {
        company_id: company_id.into(),
        fiscal_year_start: fiscal_year_start.into(),
    }
}

#[tokio::test]
async fn the_financial_statements_follow_the_vouchers() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.record_voucher(authed(sale(&id, 125_000), &anna))
        .await
        .unwrap();

    let s = api
        .get_financial_statements(authed(statements_of(&id, "2026-01-01"), &anna))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(s.previous_fiscal_year_start, "");
    assert_eq!((s.difference, s.previous_difference), (0, None));
    assert_eq!(
        s.income_statement[0],
        pb::StatementLine {
            label: "Rörelseintäkter, lagerförändringar m.m.".into(),
            kind: pb::StatementLineKind::Heading as i32,
            amount: 0,
            previous: None,
        }
    );
    assert!(s.income_statement.contains(&pb::StatementLine {
        label: "Nettoomsättning".into(),
        kind: pb::StatementLineKind::Item as i32,
        amount: 125_000,
        previous: None,
    }));
    assert!(s.income_statement.contains(&pb::StatementLine {
        label: "Rörelseresultat".into(),
        kind: pb::StatementLineKind::Subtotal as i32,
        amount: 125_000,
        previous: None,
    }));
    assert!(s.balance_sheet.contains(&pb::StatementLine {
        label: "Kassa och bank".into(),
        kind: pb::StatementLineKind::Item as i32,
        amount: 125_000,
        previous: None,
    }));
}

#[tokio::test]
async fn the_financial_statements_refuse_bad_input_and_non_members() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();

    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-13-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_date".to_owned())
    );
    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-02-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "fiscal_year_not_found".to_owned())
    );
    let err = api
        .get_financial_statements(authed(statements_of(&id, "2026-01-01"), &bo))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "company_not_found".to_owned())
    );
    let err = api
        .get_financial_statements(statements_of(&id, "2026-01-01"))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".to_owned())
    );
}

#[tokio::test]
async fn listed_vouchers_name_whoever_recorded_them() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server
        .invite_as(&anna, &mut annas, "bo@example.se", "Bo Ek")
        .await;
    let id = company(&server, &anna).await;
    server
        .add_member(&anna, &mut annas, &id, "bo@example.se")
        .await
        .unwrap();
    let mut api = server.ledger();
    api.record_voucher(authed(sale(&id, 100), &anna))
        .await
        .unwrap();
    api.record_voucher(authed(sale(&id, 200), &bo))
        .await
        .unwrap();
    api.record_voucher(authed(sale(&id, 300), &bo))
        .await
        .unwrap();

    let vouchers = api
        .list_vouchers(authed(
            pb::ListVouchersRequest {
                company_id: id,
                fiscal_year_start: "2026-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers;

    let names: Vec<&str> = vouchers
        .iter()
        .map(|v| v.recorded_by_name.as_str())
        .collect();
    assert_eq!(names, ["Anna", "Bo Ek", "Bo Ek"]);
    // The name is all of the person that leaves the server.
    assert!(!format!("{vouchers:?}").contains("example.se"));
}

async fn vat_boxes(
    api: &mut Ledger,
    session: &str,
    company_id: &str,
) -> std::collections::HashMap<u32, u32> {
    api.list_accounts(authed(
        pb::ListAccountsRequest {
            company_id: company_id.into(),
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
    .accounts
    .into_iter()
    .map(|a| (a.number, a.vat_box))
    .collect()
}

fn set_box(company_id: &str, number: u32, vat_box: u32) -> pb::SetAccountVatBoxRequest {
    pb::SetAccountVatBoxRequest {
        company_id: company_id.into(),
        number,
        vat_box,
    }
}

#[tokio::test]
async fn an_accounts_momsruta_is_listed_and_changed() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let before = vat_boxes(&mut api, &anna, &id).await;
    assert_eq!((before[&2611], before[&1930]), (10, 0));

    api.set_account_vat_box(authed(set_box(&id, 3004, 5), &anna))
        .await
        .unwrap();
    api.set_account_vat_box(authed(set_box(&id, 2611, 0), &anna))
        .await
        .unwrap();
    let after = vat_boxes(&mut api, &anna, &id).await;
    assert_eq!((after[&3004], after[&2611]), (5, 0));

    let err = api
        .set_account_vat_box(authed(set_box(&id, 2611, 49), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_vat_box".into())
    );
}

#[tokio::test]
async fn record_voucher_with_dry_run_answers_but_saves_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let mut request = sale(&id, 12_500);
    request.dry_run = true;
    request.attachments = vec![upload("kvitto.pdf", pdf(1000))];

    let preview = api
        .record_voucher(authed(request, &anna))
        .await
        .unwrap()
        .into_inner();
    let vouchers = api
        .list_vouchers(authed(
            pb::ListVouchersRequest {
                company_id: id.clone(),
                fiscal_year_start: "2026-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers;
    let real = api
        .record_voucher(authed(sale(&id, 12_500), &anna))
        .await
        .unwrap()
        .into_inner();

    assert!(preview.dry_run);
    assert_eq!(preview.number, 1);
    assert_eq!(preview.attachments.len(), 1);
    assert_eq!(preview.attachments[0].file_name, "kvitto.pdf");
    assert!(vouchers.is_empty());
    assert!(!real.dry_run);
    assert_eq!(real.number, 1);
}

#[tokio::test]
async fn a_dry_run_is_refused_like_a_real_run() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    let mut unbalanced = sale(&id, 100);
    unbalanced.lines[1].credit = 99;
    let mut dry = unbalanced.clone();
    dry.dry_run = true;

    let real = api
        .record_voucher(authed(unbalanced, &anna))
        .await
        .unwrap_err();
    let preview = api.record_voucher(authed(dry, &anna)).await.unwrap_err();

    assert_eq!(code_of(preview), code_of(real));
}

#[tokio::test]
async fn correct_voucher_with_dry_run_saves_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.ledger();
    api.record_voucher(authed(sale(&id, 100), &anna))
        .await
        .unwrap();
    let correct = |dry_run| pb::CorrectVoucherRequest {
        company_id: id.clone(),
        fiscal_year_start: "2026-01-01".into(),
        number: 1,
        date: "2026-01-16".into(),
        dry_run,
    };

    let preview = api
        .correct_voucher(authed(correct(true), &anna))
        .await
        .unwrap()
        .into_inner();
    let real = api
        .correct_voucher(authed(correct(false), &anna))
        .await
        .unwrap()
        .into_inner();

    assert!(preview.dry_run);
    assert_eq!((preview.number, real.number), (2, 2));
    assert!(!real.dry_run);
}
