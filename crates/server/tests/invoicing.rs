mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as pb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

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

fn customer(name: &str) -> pb::CustomerDetails {
    pb::CustomerDetails {
        name: name.into(),
        org_nr: "5560160680".into(),
        city: "Stockholm".into(),
        payment_terms: 30,
        ..Default::default()
    }
}

fn supplier(name: &str) -> pb::SupplierDetails {
    pb::SupplierDetails {
        name: name.into(),
        bankgiro: "50501055".into(),
        iban: "se4550000000058398257466".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_member_keeps_customers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();

    let number = api
        .add_customer(authed(
            pb::AddCustomerRequest {
                company_id: id.clone(),
                details: Some(customer("Kund AB")),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .number;
    assert_eq!(number, 1);
    api.update_customer(authed(
        pb::UpdateCustomerRequest {
            company_id: id.clone(),
            number,
            details: Some(customer("Kund i Sthlm AB")),
        },
        &anna,
    ))
    .await
    .unwrap();
    api.set_customer_active(authed(
        pb::SetCustomerActiveRequest {
            company_id: id.clone(),
            number,
            active: false,
        },
        &anna,
    ))
    .await
    .unwrap();

    let customers = api
        .list_customers(authed(
            pb::ListCustomersRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .customers;
    assert_eq!(customers.len(), 1);
    assert!(!customers[0].active);
    let details = customers[0].details.clone().unwrap();
    assert_eq!(details.name, "Kund i Sthlm AB");
    assert_eq!(details.org_nr, "556016-0680");
    assert_eq!(details.vat_number, "");
    assert_eq!(details.payment_terms, 30);
}

#[tokio::test]
async fn a_member_keeps_suppliers_and_sees_formatted_numbers() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();

    api.add_supplier(authed(
        pb::AddSupplierRequest {
            company_id: id.clone(),
            details: Some(supplier("Lev AB")),
        },
        &anna,
    ))
    .await
    .unwrap();
    api.update_supplier(authed(
        pb::UpdateSupplierRequest {
            company_id: id.clone(),
            number: 1,
            details: Some(pb::SupplierDetails {
                bic: "essesess".into(),
                ..supplier("Lev AB")
            }),
        },
        &anna,
    ))
    .await
    .unwrap();
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

    let suppliers = api
        .list_suppliers(authed(
            pb::ListSuppliersRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner()
        .suppliers;
    let details = suppliers[0].details.clone().unwrap();
    assert_eq!(details.bankgiro, "5050-1055");
    assert_eq!(details.iban, "SE45 5000 0000 0583 9825 7466");
    assert_eq!(details.bic, "ESSESESS");
    assert!(!suppliers[0].active);
}

#[tokio::test]
async fn bad_details_and_unknown_numbers_have_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();
    let invalid = |code: &str| (Code::InvalidArgument, code.to_owned());

    for (details, code) in [
        (customer(""), "invalid_name"),
        (
            pb::CustomerDetails {
                org_nr: "556016-0681".into(),
                ..customer("K")
            },
            "invalid_org_nr",
        ),
        (
            pb::CustomerDetails {
                vat_number: "SE1".into(),
                ..customer("K")
            },
            "invalid_vat_number",
        ),
        (
            pb::CustomerDetails {
                city: "å".repeat(201),
                ..customer("K")
            },
            "invalid_address",
        ),
        (
            pb::CustomerDetails {
                email: "kund".into(),
                ..customer("K")
            },
            "invalid_email",
        ),
        (
            pb::CustomerDetails {
                payment_terms: 366,
                ..customer("K")
            },
            "invalid_payment_terms",
        ),
    ] {
        let request = pb::AddCustomerRequest {
            company_id: id.clone(),
            details: Some(details),
        };
        let err = api.add_customer(authed(request, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), invalid(code));
    }
    for (details, code) in [
        (
            pb::SupplierDetails {
                bankgiro: "1".into(),
                ..supplier("L")
            },
            "invalid_bankgiro",
        ),
        (
            pb::SupplierDetails {
                plusgiro: "1".into(),
                ..supplier("L")
            },
            "invalid_plusgiro",
        ),
        (
            pb::SupplierDetails {
                iban: "SE1".into(),
                ..supplier("L")
            },
            "invalid_iban",
        ),
        (
            pb::SupplierDetails {
                bic: "X".into(),
                ..supplier("L")
            },
            "invalid_bic",
        ),
    ] {
        let request = pb::AddSupplierRequest {
            company_id: id.clone(),
            details: Some(details),
        };
        let err = api.add_supplier(authed(request, &anna)).await.unwrap_err();
        assert_eq!(code_of(err), invalid(code));
    }

    let err = api
        .update_customer(authed(
            pb::UpdateCustomerRequest {
                company_id: id.clone(),
                number: 9,
                details: Some(customer("K")),
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "customer_not_found".into()));
    let err = api
        .set_supplier_active(authed(
            pb::SetSupplierActiveRequest {
                company_id: id.clone(),
                number: 9,
                active: false,
            },
            &anna,
        ))
        .await
        .unwrap_err();
    assert_eq!(code_of(err), (Code::NotFound, "supplier_not_found".into()));
}

#[tokio::test]
async fn others_get_company_not_found_and_strangers_not_signed_in() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let id = company(&server, &anna).await;
    let mut api = server.invoicing();
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    for company_id in [id.clone(), "not-a-uuid".into()] {
        let err = api
            .list_customers(authed(
                pb::ListCustomersRequest {
                    company_id: company_id.clone(),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), not_found);
        let err = api
            .add_supplier(authed(
                pb::AddSupplierRequest {
                    company_id,
                    details: Some(supplier("L")),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), not_found);
    }

    let err = api
        .list_suppliers(pb::ListSuppliersRequest {
            company_id: id.clone(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".into())
    );
}
