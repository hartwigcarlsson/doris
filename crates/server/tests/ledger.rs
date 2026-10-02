mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as pb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company, first räkenskapsår 2026.
async fn company(server: &TestServer, session: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2026-01-01".into(),
                fiscal_year_end: "2026-12-31".into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
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
    }
}

#[tokio::test]
async fn a_member_keeps_the_chart_and_books_and_corrects_vouchers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
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
        active: true
    }));
    assert!(accounts.contains(&pb::Account {
        number: 1910,
        name: "Kassa".into(),
        active: false
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
            end: "2026-12-31".into()
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
            number: 1
        }
    );
    let corrected = api
        .correct_voucher(authed(
            pb::CorrectVoucherRequest {
                company_id: id.clone(),
                fiscal_year_start: "2026-01-01".into(),
                number: 1,
                date: "2026-01-16".into(),
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
        }
    );
    assert_eq!(
        (vouchers[1].corrects, vouchers[1].text.as_str()),
        (1, "Rättelse av ver 1")
    );
}

#[tokio::test]
async fn ledger_errors_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
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
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
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
