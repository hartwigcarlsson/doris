mod common;

use common::{Payroll, TestServer, authed, device};
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
            tax,
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
