mod common;

use common::{Invoicing, TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as pb;
use doris_proto::ledger::v1 as lpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

/// Anna's company (first year 2026) with supplier 1, "Lev AB".
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
        .add_supplier(authed(
            pb::AddSupplierRequest {
                company_id: id.clone(),
                details: Some(pb::SupplierDetails {
                    name: "Lev AB".into(),
                    bankgiro: "50501055".into(),
                    ..Default::default()
                }),
            },
            session,
        ))
        .await
        .unwrap();
    id
}

fn request(company_id: &str, invoice_number: &str) -> pb::RegisterSupplierInvoiceRequest {
    pb::RegisterSupplierInvoiceRequest {
        company_id: company_id.into(),
        supplier_number: 1,
        invoice_number: invoice_number.into(),
        invoice_date: "2026-01-15".into(),
        due_date: "2026-02-14".into(),
        reference: "".into(),
        lines: vec![pb::InvoiceLine {
            account: 5410,
            net: 80_000,
            vat_rate: 25,
        }],
        vat: None,
        attachments: vec![lpb::NewAttachment {
            file_name: "faktura.pdf".into(),
            data: b"%PDF-1.7\nfaktura".to_vec(),
        }],
    }
}

async fn list(api: &mut Invoicing, id: &str, session: &str) -> pb::ListSupplierInvoicesResponse {
    api.list_supplier_invoices(authed(
        pb::ListSupplierInvoicesRequest {
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

    let number = api
        .register_supplier_invoice(authed(request(&id, "F-4711"), &anna))
        .await
        .unwrap()
        .into_inner()
        .number;
    assert_eq!(number, 1);
    api.pay_supplier_invoice(authed(
        pb::PaySupplierInvoiceRequest {
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
    assert!(!listed.cash_method);
    let invoice = &listed.invoices[0];
    assert_eq!(invoice.supplier_name, "Lev AB");
    assert_eq!(invoice.bankgiro, "5050-1055");
    assert_eq!((invoice.vat, invoice.total), (20_000, 100_000));
    assert_eq!(invoice.status, "paid");
    assert_eq!(invoice.paid_date, "2026-01-20");
    assert_eq!(invoice.lines[0].vat_rate, 25);
    assert_eq!(invoice.vouchers.len(), 2);
    assert_eq!(invoice.vouchers[0].fiscal_year_start, "2026-01-01");
    assert_eq!(invoice.attachments[0].file_name, "faktura.pdf");

    api.reverse_supplier_invoice_payment(authed(
        pb::ReverseSupplierInvoicePaymentRequest {
            company_id: id.clone(),
            number,
            reason: "Fel konto".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(
        list(&mut api, &id, &anna).await.invoices[0].status,
        "unpaid"
    );

    let sha = invoice.attachments[0].id.clone();
    let file = api
        .get_supplier_invoice_attachment(authed(
            pb::GetSupplierInvoiceAttachmentRequest {
                company_id: id.clone(),
                number,
                sha256: sha,
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(file.data, b"%PDF-1.7\nfaktura");
    assert_eq!(file.attachment.unwrap().content_type, "application/pdf");
}

#[tokio::test]
async fn kontantmetoden_is_reported_and_cancelling_works() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Cash).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna))
        .await
        .unwrap();

    let listed = list(&mut api, &id, &anna).await;
    assert!(listed.cash_method);
    assert!(listed.invoices[0].vouchers.is_empty());
    api.cancel_supplier_invoice(authed(
        pb::CancelSupplierInvoiceRequest {
            company_id: id.clone(),
            number: 1,
            reason: "Dubbel".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    assert_eq!(
        list(&mut api, &id, &anna).await.invoices[0].status,
        "cancelled"
    );
}

#[tokio::test]
async fn bad_requests_have_stable_codes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    let ok = || request(&id, "F-1");
    let line = |account, net, vat_rate| pb::InvoiceLine {
        account,
        net,
        vat_rate,
    };

    for (req, code, expected) in [
        (
            pb::RegisterSupplierInvoiceRequest {
                invoice_number: "".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_invoice_number",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                due_date: "2026-01-01".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_due_date",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                reference: "1".repeat(51),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_reference",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                lines: vec![],
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_invoice_lines",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                lines: vec![line(5410, 100, 20)],
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_vat_rate",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                lines: vec![line(2440, 100, 25)],
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_invoice_account",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                vat: Some(25_000),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_vat_amount",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                invoice_date: "2026-13-01".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "invalid_date",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                invoice_date: "2099-01-01".into(),
                due_date: "2099-02-01".into(),
                ..ok()
            },
            Code::InvalidArgument,
            "voucher_date_in_future",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                supplier_number: 9,
                ..ok()
            },
            Code::NotFound,
            "supplier_not_found",
        ),
        (
            pb::RegisterSupplierInvoiceRequest {
                lines: vec![line(1931, 100, 25)],
                ..ok()
            },
            Code::NotFound,
            "account_not_found",
        ),
    ] {
        let err = api
            .register_supplier_invoice(authed(req, &anna))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (code, expected.to_owned()));
    }

    api.register_supplier_invoice(authed(ok(), &anna))
        .await
        .unwrap();
    let err = api
        .register_supplier_invoice(authed(ok(), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::AlreadyExists, "duplicate_supplier_invoice".into())
    );

    let pay = |number, account| pb::PaySupplierInvoiceRequest {
        company_id: id.clone(),
        number,
        date: "2026-01-20".into(),
        account,
    };
    let err = api
        .pay_supplier_invoice(authed(pay(1, 2440), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_payment_account".into())
    );
    let err = api
        .pay_supplier_invoice(authed(pay(9, 1930), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "supplier_invoice_not_found".into())
    );
    let reverse = |reason: &str| pb::ReverseSupplierInvoicePaymentRequest {
        company_id: id.clone(),
        number: 1,
        reason: reason.into(),
    };
    let err = api
        .reverse_supplier_invoice_payment(authed(reverse("Fel"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "supplier_invoice_not_paid".into())
    );
    let cancel = |reason: &str| pb::CancelSupplierInvoiceRequest {
        company_id: id.clone(),
        number: 1,
        reason: reason.into(),
    };
    let err = api
        .cancel_supplier_invoice(authed(cancel(""), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_reason".into())
    );
    api.pay_supplier_invoice(authed(pay(1, 1930), &anna))
        .await
        .unwrap();
    let err = api
        .cancel_supplier_invoice(authed(cancel("Fel"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "supplier_invoice_paid".into())
    );

    api.set_supplier_active(authed(
        pb::SetSupplierActiveRequest {
            company_id: id.clone(),
            number: 1,
            active: false,
        },
        &anna,
    ))
    .await
    .unwrap();
    let err = api
        .register_supplier_invoice(authed(request(&id, "F-2"), &anna))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "supplier_inactive".into())
    );
}

#[tokio::test]
async fn a_cancelled_invoice_cannot_be_paid() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna))
        .await
        .unwrap();
    api.cancel_supplier_invoice(authed(
        pb::CancelSupplierInvoiceRequest {
            company_id: id.clone(),
            number: 1,
            reason: "Dubbel".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let err = api
        .pay_supplier_invoice(authed(
            pb::PaySupplierInvoiceRequest {
                company_id: id.clone(),
                number: 1,
                date: "2026-01-20".into(),
                account: 1930,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (
            Code::FailedPrecondition,
            "supplier_invoice_cancelled".into()
        )
    );
}

#[tokio::test]
async fn others_cannot_see_the_invoices_or_their_underlag() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna, cpb::AccountingMethod::Invoice).await;
    let mut api = server.invoicing();
    api.register_supplier_invoice(authed(request(&id, "F-1"), &anna))
        .await
        .unwrap();
    let sha = list(&mut api, &id, &anna).await.invoices[0].attachments[0]
        .id
        .clone();

    let err = api
        .list_supplier_invoices(authed(
            pb::ListSupplierInvoicesRequest {
                company_id: id.clone(),
            },
            &bo,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    let err = api
        .get_supplier_invoice_attachment(authed(
            pb::GetSupplierInvoiceAttachmentRequest {
                company_id: id.clone(),
                number: 2,
                sha256: sha,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::NotFound, "attachment_not_found".into())
    );
}

#[tokio::test]
async fn a_huge_invoicing_frame_without_a_session_is_refused_before_its_body_is_read() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = TestServer::start().await;
    let claimed: u32 = 21 << 20;
    let mut header = vec![0u8];
    header.extend_from_slice(&claimed.to_be_bytes());
    let mut stream = tokio::net::TcpStream::connect(server.base.trim_start_matches("http://"))
        .await
        .unwrap();
    let request = format!(
        "POST /doris.invoicing.v1.InvoicingService/RegisterSupplierInvoice HTTP/1.1\r\n\
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
    assert!(head.contains("grpc-message: not_signed_in"), "{head}");
}
