mod common;

use common::{Payroll, TestServer, authed, device, fake_skatteverket, tax_rows};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::payroll::v1 as pb;
use tonic::{Code, Request};

const KR: i64 = 100;

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

async fn hire(api: &mut Payroll, session: &str, company_id: &str) -> String {
    api.add_employee(authed(
        pb::AddEmployeeRequest {
            company_id: company_id.into(),
            name: "Åsa Öberg".into(),
            personal_identity_number: "19800101-1231".into(),
            monthly_salary: 35_000 * KR,
            salary_account: 7210,
            tax: None,
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
    .employee_id
}

fn draft(pay_date: &str, employee_id: &str, gross: i64, tax: i64) -> Option<pb::PayrollRunDraft> {
    Some(pb::PayrollRunDraft {
        pay_date: pay_date.into(),
        text: String::new(),
        lines: vec![pb::PayrollRunLineInput {
            employee_id: employee_id.into(),
            gross,
            tax: Some(tax),
        }],
    })
}

fn run_ref(company_id: &str, run: &str) -> pb::PayrollRunRef {
    pb::PayrollRunRef {
        company_id: company_id.into(),
        payroll_run_id: run.into(),
    }
}

#[tokio::test]
async fn a_member_runs_payroll_from_employee_to_voucher_and_back() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;

    let employees = api
        .list_employees(authed(
            pb::ListEmployeesRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .employees;
    assert_eq!(employees[0].personal_identity_number, "19800101-1231");

    let preview = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.text, "Lön januari 2026");
    assert_eq!(preview.lines[0].employee_name, "Åsa Öberg");
    assert_eq!(
        (preview.lines[0].fee_rate, preview.lines[0].fee),
        (3142, 1_099_700)
    );
    assert_eq!(preview.voucher_lines.len(), 5);

    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();
    let booked = api
        .book_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (booked.fiscal_year_start.as_str(), booked.number),
        ("2026-01-01", 1)
    );

    let vouchers = server
        .ledger()
        .list_vouchers(authed(
            lpb::ListVouchersRequest {
                company_id: id.clone(),
                fiscal_year_start: "2026-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .vouchers;
    assert_eq!(vouchers[0].text, "Lön januari 2026");
    let shown = api
        .get_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(shown.status(), pb::PayrollRunStatus::Booked);
    assert_eq!(shown.voucher.unwrap().number, 1);

    let correction = api
        .unbook_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(correction.number, 2);
    api.reopen_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();
    api.update_payroll_run(authed(
        pb::UpdatePayrollRunRequest {
            run: Some(run_ref(&id, &run)),
            draft: draft("2026-01-25", &asa, 36_000 * KR, 8_300 * KR),
        },
        &anna,
    ))
    .await
    .unwrap();
    let runs = api
        .list_payroll_runs(authed(
            pb::ListPayrollRunsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_runs;
    assert_eq!(runs[0].status(), pb::PayrollRunStatus::Open);
    assert_eq!(runs[0].lines[0].gross, 36_000 * KR);
}

#[tokio::test]
async fn the_lifecycle_and_the_pay_date_are_enforced() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;
    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2099-12-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();

    let early = api
        .book_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(early),
        (Code::FailedPrecondition, "payroll_run_not_due".into())
    );
    let locked = api
        .update_payroll_run(authed(
            pb::UpdatePayrollRunRequest {
                run: Some(run_ref(&id, &run)),
                draft: draft("2099-12-25", &asa, 1, 0),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(locked),
        (Code::FailedPrecondition, "payroll_run_not_open".into())
    );
    let missing = api
        .get_payroll_run(authed(run_ref(&id, "not-a-uuid"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(missing),
        (Code::NotFound, "payroll_run_not_found".into())
    );
}

#[tokio::test]
async fn invalid_input_gets_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;

    let bad_pin = api
        .add_employee(authed(
            pb::AddEmployeeRequest {
                company_id: id.clone(),
                name: "Bo".into(),
                personal_identity_number: "19800101-1232".into(),
                monthly_salary: 1,
                salary_account: 7210,
                tax: None,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad_pin),
        (
            Code::InvalidArgument,
            "invalid_personal_identity_number".into()
        )
    );
    let twice = api
        .add_employee(authed(
            pb::AddEmployeeRequest {
                company_id: id.clone(),
                name: "Åsa".into(),
                personal_identity_number: "198001011231".into(),
                monthly_salary: 1,
                salary_account: 7210,
                tax: None,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(twice),
        (Code::FailedPrecondition, "duplicate_employee".into())
    );
    let mut long = draft("2026-01-25", &asa, 100, 0).unwrap();
    long.text = "x".repeat(201);
    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: Some(long),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(refused),
        (Code::InvalidArgument, "invalid_voucher_text".into())
    );
    let bad_tax = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 100, 101),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad_tax),
        (Code::InvalidArgument, "invalid_tax".into())
    );
}

#[tokio::test]
async fn strangers_and_signed_out_callers_find_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.payroll();

    let stranger = api
        .list_employees(authed(
            pb::ListEmployeesRequest {
                company_id: id.clone(),
            },
            &bo,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(stranger),
        (Code::NotFound, "company_not_found".into())
    );
    let signed_out = api
        .list_payroll_runs(Request::new(pb::ListPayrollRunsRequest { company_id: id }))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(signed_out),
        (Code::Unauthenticated, "not_signed_in".into())
    );
}

use std::sync::atomic::Ordering;

fn table_33() -> Option<pb::TaxSetting> {
    Some(pb::TaxSetting {
        kind: Some(pb::tax_setting::Kind::Table(pb::TableTax {
            table: 33,
            column: 1,
        })),
    })
}

async fn hire_with(
    api: &mut Payroll,
    session: &str,
    company_id: &str,
    pin: &str,
    tax: Option<pb::TaxSetting>,
) -> String {
    api.add_employee(authed(
        pb::AddEmployeeRequest {
            company_id: company_id.into(),
            name: "Åsa Öberg".into(),
            personal_identity_number: pin.into(),
            monthly_salary: 35_000 * KR,
            salary_account: 7210,
            tax,
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
    .employee_id
}

fn computed(pay_date: &str, employee_id: &str) -> Option<pb::PayrollRunDraft> {
    Some(pb::PayrollRunDraft {
        pay_date: pay_date.into(),
        text: String::new(),
        lines: vec![pb::PayrollRunLineInput {
            employee_id: employee_id.into(),
            gross: 35_000 * KR,
            tax: None,
        }],
    })
}

#[tokio::test]
async fn a_table_employees_tax_is_fetched_once_and_computed() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let preview = async |api: &mut Payroll| {
        api.preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2026-01-25", &asa),
            },
            &anna,
        ))
        .await
    };
    let line = preview(&mut api)
        .await
        .unwrap()
        .into_inner()
        .lines
        .remove(0);
    assert_eq!(line.tax, Some(7_134 * KR));
    assert_eq!(
        line.tax_basis.and_then(|b| b.kind),
        Some(pb::tax_basis::Kind::Table(pb::TableBasis {
            year: 2026,
            table: 33,
            column: 1
        }))
    );
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3);

    preview(&mut api).await.unwrap();
    assert_eq!(
        fake.requests.load(Ordering::SeqCst),
        3,
        "the stored year is reused"
    );
    let employees = api
        .list_employees(authed(
            pb::ListEmployeesRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .employees;
    assert_eq!(employees[0].tax, table_33());
}

#[tokio::test]
async fn without_skatteverket_a_computed_tax_is_unavailable_but_a_typed_one_works() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.broken.store(true, Ordering::SeqCst);
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2026-01-25", &asa),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(refused),
        (Code::Unavailable, "tax_table_unavailable".into())
    );

    let typed = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(typed.lines[0].tax, Some(8_000 * KR));
    assert_eq!(
        typed.lines[0].tax_basis.and_then(|b| b.kind),
        Some(pb::tax_basis::Kind::Manual(true))
    );
}

#[tokio::test]
async fn a_year_skatteverket_has_not_published_is_unavailable() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;

    let refused = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2027-01-25", &asa),
            },
            &anna,
        ))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(refused),
        (Code::Unavailable, "tax_table_unavailable".into())
    );
}

#[tokio::test]
async fn tax_settings_are_checked_and_a_blank_tax_needs_one() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", None).await;
    let set = |tax: pb::tax_setting::Kind| pb::SetEmployeeTaxRequest {
        company_id: id.clone(),
        employee_id: asa.clone(),
        tax: Some(pb::TaxSetting { kind: Some(tax) }),
    };

    let blank = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2026-01-25", &asa),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(blank),
        (Code::InvalidArgument, "tax_required".into())
    );

    let bad = api
        .set_employee_tax(authed(
            set(pb::tax_setting::Kind::Table(pb::TableTax {
                table: 43,
                column: 1,
            })),
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad),
        (Code::InvalidArgument, "invalid_tax_table".into())
    );
    let bad = api
        .set_employee_tax(authed(set(pb::tax_setting::Kind::Percent(101)), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad),
        (Code::InvalidArgument, "invalid_tax_percent".into())
    );

    api.set_employee_tax(authed(set(pb::tax_setting::Kind::Percent(30)), &anna))
        .await
        .unwrap();
    // A percentage needs no table, so no Skatteverket either.
    let line = api
        .preview_payroll_run(authed(
            pb::PreviewPayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2026-01-25", &asa),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .lines
        .remove(0);
    assert_eq!(line.tax, Some(10_500 * KR));
}

#[tokio::test]
async fn finalizing_a_blank_tax_run_fetches_the_year_and_locks_the_computed_tax() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let server = TestServer::start_with_tax_tables(&fake.url).await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire_with(&mut api, &anna, &id, "19800101-1231", table_33()).await;
    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: computed("2026-01-25", &asa),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    assert_eq!(fake.requests.load(Ordering::SeqCst), 0);

    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();

    assert!(fake.requests.load(Ordering::SeqCst) > 0);
    let shown = api
        .get_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(shown.status(), pb::PayrollRunStatus::Finalized);
    let line = &shown.lines[0];
    assert_eq!(line.tax, Some(7_134 * KR));
    assert_eq!(
        line.tax_basis.and_then(|b| b.kind),
        Some(pb::tax_basis::Kind::Table(pb::TableBasis {
            year: 2026,
            table: 33,
            column: 1
        }))
    );
}

fn agi_ref(company_id: &str, period: &str) -> pb::AgiMonthRef {
    pb::AgiMonthRef {
        company_id: company_id.into(),
        period: period.into(),
    }
}

fn mark(company_id: &str, period: &str, fingerprint: &str) -> pb::MarkAgiSubmittedRequest {
    pb::MarkAgiSubmittedRequest {
        company_id: company_id.into(),
        period: period.into(),
        fingerprint: fingerprint.into(),
    }
}

#[tokio::test]
async fn a_month_is_declared_and_corrected() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();
    let asa = hire(&mut api, &anna, &id).await;
    let run = api
        .create_payroll_run(authed(
            pb::CreatePayrollRunRequest {
                company_id: id.clone(),
                draft: draft("2026-01-25", &asa, 35_000 * KR, 8_000 * KR),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .payroll_run_id;
    api.finalize_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();
    api.book_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();

    let months = api
        .list_agi_months(authed(
            pb::ListAgiMonthsRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .months;
    assert_eq!(months[0].period, "202601");
    assert_eq!(
        (months[0].gross, months[0].tax_sum, months[0].fee_sum),
        (35_000, 8_000, 10_997)
    );
    assert_eq!(months[0].status(), pb::AgiStatus::NotSubmitted);

    let missing = api
        .export_agi_file(authed(agi_ref(&id, "202601"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(missing),
        (Code::FailedPrecondition, "agi_contact_missing".into())
    );
    let contact = pb::AgiContact {
        name: "Anna Andersson".into(),
        phone: "070-123 45 67".into(),
        email: "anna@example.se".into(),
    };
    api.set_agi_contact(authed(
        pb::SetAgiContactRequest {
            company_id: id.clone(),
            contact: Some(contact.clone()),
        },
        &anna,
    ))
    .await
    .unwrap();
    let got = api
        .get_agi_contact(authed(
            pb::GetAgiContactRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got, contact);

    let file = api
        .export_agi_file(authed(agi_ref(&id, "202601"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.file_name, "AGI_165560160680_202601.xml");
    assert!(
        file.xml
            .contains(r#"<agd:SummaSkatteavdr faltkod="497">8000</agd:SummaSkatteavdr>"#)
    );
    assert!(file.xml.contains("198001011231"));

    let shown = api
        .get_agi_month(authed(agi_ref(&id, "202601"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(shown.fingerprint, file.fingerprint);
    assert_eq!(file.fingerprint.len(), 64);
    api.mark_agi_submitted(authed(mark(&id, "202601", &file.fingerprint), &anna))
        .await
        .unwrap();
    let again = api
        .mark_agi_submitted(authed(mark(&id, "202601", &file.fingerprint), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(again),
        (Code::FailedPrecondition, "agi_unchanged".into())
    );

    api.unbook_payroll_run(authed(run_ref(&id, &run), &anna))
        .await
        .unwrap();
    let outdated = api
        .mark_agi_submitted(authed(mark(&id, "202601", &file.fingerprint), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(outdated),
        (Code::FailedPrecondition, "agi_file_outdated".into())
    );
    let month = api
        .get_agi_month(authed(agi_ref(&id, "202601"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(month.summary.unwrap().status(), pb::AgiStatus::Changed);
    assert_eq!(month.lines[0].change(), pb::AgiChange::Removed);
    assert_eq!(month.lines[0].personal_identity_number, "19800101-1231");
    let file = api
        .export_agi_file(authed(agi_ref(&id, "202601"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert!(
        file.xml
            .contains(r#"<agd:Borttag faltkod="205">1</agd:Borttag>"#)
    );
}

#[tokio::test]
async fn agi_input_is_checked() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.payroll();

    let bad = api
        .get_agi_month(authed(agi_ref(&id, "2026-01"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad),
        (Code::InvalidArgument, "invalid_period".into())
    );
    let contact = pb::AgiContact {
        name: "Anna".into(),
        phone: "".into(),
        email: "anna@example.se".into(),
    };
    let bad = api
        .set_agi_contact(authed(
            pb::SetAgiContactRequest {
                company_id: id.clone(),
                contact: Some(contact),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(bad),
        (Code::InvalidArgument, "invalid_agi_contact".into())
    );
    let empty = api
        .mark_agi_submitted(authed(mark(&id, "202601", ""), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(empty),
        (Code::FailedPrecondition, "agi_period_empty".into())
    );
    let none = api
        .get_agi_contact(authed(
            pb::GetAgiContactRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(none, pb::AgiContact::default());
}
