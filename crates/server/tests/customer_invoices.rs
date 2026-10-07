mod common;

use common::{Invoicing, TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as pb;
use doris_proto::ledger::v1 as lpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company (first year 2026) with customer 1, "Kund AB".
async fn company(server: &TestServer, session: &str, method: cpb::AccountingMethod) -> String {
    let id = server
        .companies()
        .create_company(authed(
            cpb::CreateCompanyRequest {
                org_nr: "556016-0680".into(),
                name: "Exempel AB".into(),
                legal_form: cpb::LegalForm::Aktiebolag as i32,
                address: None,
                fiscal_year_start: "2026-01-01".into(),
                fiscal_year_end: "2026-12-31".into(),
                accounting_method: method as i32,
            },
            session,
        ))
        .await
        .unwrap()
        .into_inner()
        .company_id;
    server
        .invoicing()
        .add_customer(authed(
            pb::AddCustomerRequest {
                company_id: id.clone(),
                details: Some(pb::CustomerDetails {
                    name: "Kund AB".into(),
                    payment_terms: 30,
                    ..Default::default()
                }),
            },
            session,
        ))
        .await
        .unwrap();
    id
}

fn request(company_id: &str, invoice_number: &str) -> pb::RegisterCustomerInvoiceRequest {
    pb::RegisterCustomerInvoiceRequest {
        company_id: company_id.into(),
        customer_number: 1,
        invoice_number: invoice_number.into(),
        invoice_date: "2026-01-15".into(),
        due_date: "2026-02-14".into(),
        reference: "".into(),
        lines: vec![
            pb::InvoiceLine {
                account: 3001,
                net: 80_000,
                vat_rate: 25,
            },
            pb::InvoiceLine {
                account: 3002,
                net: 10_000,
                vat_rate: 12,
            },
        ],
        attachments: vec![lpb::NewAttachment {
            file_name: "faktura.pdf".into(),
            data: b"%PDF-1.7\nfaktura".to_vec(),
        }],
    }
}

async fn list(api: &mut Invoicing, id: &str, session: &str) -> pb::ListCustomerInvoicesResponse {
    api.list_customer_invoices(authed(
        pb::ListCustomerInvoicesRequest {
            company_id: id.into(),
        },
        session,
    ))
    .await
    .unwrap()
    .into_inner()
}

#[tokio::test]
async fn an_invoice_is_registered_paid_reversed_and_listed() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    assert_eq!(list(&mut api, &id, &anna).await.next_invoice_number, "1");

    let number = api
        .register_customer_invoice(authed(request(&id, "1017"), &anna))
        .await
        .unwrap()
        .into_inner()
        .number;
    api.pay_customer_invoice(authed(
        pb::PayCustomerInvoiceRequest {
            company_id: id.clone(),
            number,
            date: "2026-01-20".into(),
            account: 1930,
        },
        &anna,
    ))
    .await
    .unwrap();

    let listed = list(&mut api, &id, &anna).await;
    assert_eq!(listed.next_invoice_number, "1018");
    assert!(!listed.cash_method);
    let invoice = &listed.invoices[0];
    assert_eq!(invoice.invoice_number, "1017");
    assert_eq!(invoice.customer_name, "Kund AB");
    let vat: Vec<_> = invoice.vat.iter().map(|v| (v.vat_rate, v.amount)).collect();
    assert_eq!(vat, [(25, 20_000), (12, 1_200)]);
    assert_eq!(invoice.total, 111_200);
    assert_eq!(
        (invoice.status.as_str(), invoice.paid_date.as_str()),
        ("paid", "2026-01-20")
    );
    assert_eq!(invoice.vouchers.len(), 2);

    api.reverse_customer_invoice_payment(authed(
        pb::ReverseCustomerInvoicePaymentRequest {
            company_id: id.clone(),
            number,
            reason: "Fel".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(
        list(&mut api, &id, &anna).await.invoices[0].status,
        "unpaid"
    );
    let file = api
        .get_customer_invoice_attachment(authed(
            pb::GetCustomerInvoiceAttachmentRequest {
                company_id: id.clone(),
                number,
                sha256: invoice.attachments[0].id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.data, b"%PDF-1.7\nfaktura");
}

#[tokio::test]
async fn bad_requests_have_stable_codes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    let ok = || request(&id, "1");

    for (req, code, expected) in [
        (
            pb::RegisterCustomerInvoiceRequest {
                customer_number: 9,
                ..ok()
            },
            Code::NotFound,
            "customer_not_found",
        ),
        (
            pb::RegisterCustomerInvoiceRequest {
                lines: vec![pb::InvoiceLine {
                    account: 1510,
                    net: 100,
                    vat_rate: 25,
                }],
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_invoice_account",
        ),
        (
            pb::RegisterCustomerInvoiceRequest {
                invoice_number: "".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_invoice_number",
        ),
        (
            pb::RegisterCustomerInvoiceRequest {
                due_date: "2026-01-01".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_due_date",
        ),
    ] {
        let err = api
            .register_customer_invoice(authed(req, &anna))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (code, expected.to_owned()));
    }

    api.register_customer_invoice(authed(ok(), &anna))
        .await
        .unwrap();
    api.cancel_customer_invoice(authed(
        pb::CancelCustomerInvoiceRequest {
            company_id: id.clone(),
            number: 1,
            reason: "Fel".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let err = api
        .register_customer_invoice(authed(ok(), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::AlreadyExists, "duplicate_customer_invoice".into())
    );
    let pay = |number| pb::PayCustomerInvoiceRequest {
        company_id: id.clone(),
        number,
        date: "2026-01-20".into(),
        account: 1930,
    };
    let err = api
        .pay_customer_invoice(authed(pay(1), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (
            Code::FailedPrecondition,
            "customer_invoice_cancelled".into()
        )
    );
    let err = api
        .pay_customer_invoice(authed(pay(9), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "customer_invoice_not_found".into())
    );

    api.register_customer_invoice(authed(request(&id, "2"), &anna))
        .await
        .unwrap();
    let err = api
        .reverse_customer_invoice_payment(authed(
            pb::ReverseCustomerInvoicePaymentRequest {
                company_id: id.clone(),
                number: 2,
                reason: "Fel".into(),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "customer_invoice_not_paid".into())
    );
    api.pay_customer_invoice(authed(pay(2), &anna))
        .await
        .unwrap();
    let err = api
        .cancel_customer_invoice(authed(
            pb::CancelCustomerInvoiceRequest {
                company_id: id.clone(),
                number: 2,
                reason: "Fel".into(),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "customer_invoice_paid".into())
    );

    api.set_customer_active(authed(
        pb::SetCustomerActiveRequest {
            company_id: id.clone(),
            number: 1,
            active: false,
        },
        &anna,
    ))
    .await
    .unwrap();
    let err = api
        .register_customer_invoice(authed(request(&id, "3"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "customer_inactive".into())
    );
}

#[tokio::test]
async fn kontantmetoden_is_reported_and_others_are_refused() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna, cpb::AccountingMethod::Cash).await;
    let mut api = server.invoicing();
    api.register_customer_invoice(authed(request(&id, "1"), &anna))
        .await
        .unwrap();
    let listed = list(&mut api, &id, &anna).await;
    assert!(listed.cash_method);
    assert!(listed.invoices[0].vouchers.is_empty());

    let err = api
        .list_customer_invoices(authed(
            pb::ListCustomerInvoicesRequest {
                company_id: id.clone(),
            },
            &bo,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api
        .list_customer_invoices(pb::ListCustomerInvoicesRequest {
            company_id: id.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".into())
    );
}
