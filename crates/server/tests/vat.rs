mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::vat::v1 as pb;
use tonic::{Code, Request};

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company, first räkenskapsår 2025, so its periods have ended.
async fn company(server: &TestServer, session: &str) -> String {
    server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2025-01-01".into(),
                fiscal_year_end: "2025-12-31".into(),
                accounting_method: cpb::AccountingMethod::Invoice as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id
}

fn r(company_id: &str, period: &str) -> pb::VatReturnRef {
    pb::VatReturnRef {
        company_id: company_id.into(),
        period: period.into(),
    }
}

fn mark(company_id: &str, period: &str, fingerprint: &str) -> pb::MarkVatReturnSubmittedRequest {
    pb::MarkVatReturnSubmittedRequest {
        company_id: company_id.into(),
        period: period.into(),
        fingerprint: fingerprint.into(),
    }
}

fn line(account: u32, debit: i64, credit: i64) -> lpb::VoucherLine {
    lpb::VoucherLine {
        account,
        debit,
        credit,
    }
}

#[tokio::test]
async fn a_member_declares_a_quarter_and_the_settlement_is_booked() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    server
        .ledger()
        .record_voucher(authed(
            lpb::RecordVoucherRequest {
                company_id: id.clone(),
                date: "2025-11-10".into(),
                text: "Försäljning".into(),
                lines: vec![
                    line(1930, 125_000, 0),
                    line(3001, 0, 100_000),
                    line(2611, 0, 25_000),
                ],
                attachments: vec![],
            },
            &anna,
        ))
        .await
        .unwrap();
    let mut api = server.vat();
    let year = api
        .list_vat_returns(authed(
            pb::ListVatReturnsRequest {
                company_id: id.clone(),
                fiscal_year_start: "2025-01-01".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(year.kind(), pb::VatPeriodKind::Quarterly);
    let q4 = year.periods.last().unwrap();
    assert_eq!(
        (
            q4.period.as_str(),
            q4.label.as_str(),
            q4.status(),
            q4.vat_due,
            q4.due_date.as_str()
        ),
        (
            "202512",
            "oktober–december 2025",
            pb::VatStatus::ToSubmit,
            250,
            "2026-02-12"
        )
    );

    let declaration = api
        .get_vat_return(authed(r(&id, "202512"), &anna))
        .await
        .unwrap()
        .into_inner();
    let boxes: Vec<(u32, i64)> = declaration
        .boxes
        .iter()
        .map(|b| (b.r#box, b.amount))
        .collect();
    assert_eq!(boxes, [(5, 1_000), (10, 250)]);
    assert_eq!(declaration.boxes[0].accounts[0].amount, 100_000);
    assert_eq!(declaration.vat_number, "SE556016068001");

    let file = api
        .export_vat_file(authed(r(&id, "202512"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.file_name, "moms_5560160680_202512.xml");
    assert!(file.content.contains("<MomsUtgHog>250</MomsUtgHog>"));

    let marked = api
        .mark_vat_return_submitted(authed(mark(&id, "202512", &file.fingerprint), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (marked.fiscal_year_start.as_str(), marked.voucher_number),
        ("2025-01-01", 2)
    );
    let again = api
        .mark_vat_return_submitted(authed(mark(&id, "202512", &file.fingerprint), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(again),
        (Code::FailedPrecondition, "vat_return_unchanged".into())
    );

    let after = api
        .get_vat_return(authed(r(&id, "202512"), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after.summary.unwrap().status(), pb::VatStatus::Submitted);
    assert_eq!(
        (
            after.submissions[0].submitted_by_name.as_str(),
            after.submissions[0].voucher_number
        ),
        ("Anna", 2)
    );
}

#[tokio::test]
async fn vat_input_gets_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.vat();
    for period in ["2025", "202513", "202511", "abcdef"] {
        let err = api
            .get_vat_return(authed(r(&id, period), &anna))
            .await
            .unwrap_err();
        assert_eq!(
            code_of(err),
            (Code::InvalidArgument, "invalid_vat_period".into()),
            "{period}"
        );
    }
    let err = api
        .mark_vat_return_submitted(authed(mark(&id, "202503", "stale"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "vat_return_outdated".into())
    );
    let fingerprint = api
        .get_vat_return(authed(r(&id, "202503"), &anna))
        .await
        .unwrap()
        .into_inner()
        .fingerprint;
    let marked = api
        .mark_vat_return_submitted(authed(mark(&id, "202503", &fingerprint), &anna))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(marked.voucher_number, 0, "nothing to book");

    let set = |kind: i32| pb::SetVatPeriodRequest {
        company_id: id.clone(),
        fiscal_year_start: "2025-01-01".into(),
        kind,
    };
    let err = api
        .set_vat_period(authed(set(pb::VatPeriodKind::Monthly as i32), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "vat_period_locked".into())
    );
    let err = api.set_vat_period(authed(set(0), &anna)).await.unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_vat_period".into())
    );
}

#[tokio::test]
async fn strangers_and_signed_out_callers_find_nothing() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    // An invited user who is not a member of Anna's company.
    let bertil = server.invite(&anna, "bertil@example.se").await;
    let mut api = server.vat();
    let err = api
        .get_vat_return(authed(r(&id, "202503"), &bertil))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api
        .get_vat_return(Request::new(r(&id, "202503")))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".into())
    );
}
