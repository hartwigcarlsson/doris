mod common;

use common::{TestServer, authed, device};
use doris_proto::company::v1 as pb;
use tonic::Code;

fn create(org_nr: &str, name: &str) -> pb::CreateCompanyRequest {
    pb::CreateCompanyRequest {
        org_nr: org_nr.into(),
        name: name.into(),
        legal_form: pb::LegalForm::Aktiebolag as i32,
        address: Some(pb::Address {
            street: "".into(),
            postal_code: "111 22".into(),
            city: "Stockholm".into(),
        }),
        fiscal_year_start: "2026-01-01".into(),
        fiscal_year_end: "2026-12-31".into(),
        accounting_method: pb::AccountingMethod::Invoice as i32,
    }
}

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

#[tokio::test]
async fn a_user_creates_lists_and_opens_a_company() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();

    let id = api
        .create_company(authed(create("556016-0680", "Exempel AB"), &anna))
        .await
        .unwrap()
        .into_inner()
        .company_id;
    let list = api
        .list_companies(authed(pb::ListCompaniesRequest {}, &anna))
        .await
        .unwrap()
        .into_inner();
    let company = api
        .get_company(authed(
            pb::GetCompanyRequest {
                company_id: id.clone(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        list.companies,
        vec![pb::CompanySummary {
            id: id.clone(),
            org_nr: "556016-0680".into(),
            name: "Exempel AB".into()
        }]
    );
    assert_eq!(company.org_nr, "556016-0680");
    assert_eq!(company.legal_form(), pb::LegalForm::Aktiebolag);
    assert_eq!(company.accounting_method(), pb::AccountingMethod::Invoice);
    assert_eq!(company.address.unwrap().city, "Stockholm");
    // The current räkenskapsår is a whole calendar year at or after the first.
    assert!(
        company.fiscal_year_start.ends_with("-01-01")
            && company.fiscal_year_end.ends_with("-12-31")
    );
    assert!(company.fiscal_year_start.as_str() >= "2026-01-01");
}

#[tokio::test]
async fn create_rejects_invalid_input_with_stable_codes() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut api = server.companies();
    let mut unparseable = create("556016-0680", "Exempel AB");
    unparseable.fiscal_year_end = "31/12".into();
    let mut no_form = create("556016-0680", "Exempel AB");
    no_form.legal_form = 0;
    let mut no_method = create("556016-0680", "Exempel AB");
    no_method.accounting_method = 0;
    let mut broken_hb = create("556016-0680", "Exempel HB");
    broken_hb.legal_form = pb::LegalForm::Handelsbolag as i32;
    broken_hb.fiscal_year_start = "2026-05-01".into();
    broken_hb.fiscal_year_end = "2027-04-30".into();

    for (request, code) in [
        (create("556016-0681", "Exempel AB"), "invalid_org_nr"),
        (create("556016-0680", ""), "invalid_company_name"),
        (unparseable, "invalid_fiscal_year"),
        (broken_hb, "invalid_fiscal_year"),
        (no_form, "invalid_legal_form"),
        (no_method, "invalid_accounting_method"),
    ] {
        let err = api
            .create_company(authed(request, &anna))
            .await
            .unwrap_err();
        assert_eq!(code_of(err), (Code::InvalidArgument, code.into()));
    }
    api.create_company(authed(create("556016-0680", "Exempel AB"), &anna))
        .await
        .unwrap();
    let dup = api
        .create_company(authed(create("5560160680", "Igen AB"), &anna))
        .await
        .unwrap_err();
    assert_eq!(code_of(dup), (Code::AlreadyExists, "company_exists".into()));
}

#[tokio::test]
async fn every_company_rpc_needs_a_session() {
    let server = TestServer::start().await;
    let err = server
        .companies()
        .list_companies(pb::ListCompaniesRequest {})
        .await
        .unwrap_err();
    assert_eq!(
        code_of(err),
        (Code::Unauthenticated, "not_signed_in".into())
    );
}

#[tokio::test]
async fn a_member_adds_a_colleague_who_then_sees_the_company() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.companies();
    let id = api
        .create_company(authed(create("556016-0680", "Exempel AB"), &anna))
        .await
        .unwrap()
        .into_inner()
        .company_id;

    let before = api
        .list_companies(authed(pb::ListCompaniesRequest {}, &bo))
        .await
        .unwrap()
        .into_inner();
    api.add_member(authed(
        pb::AddMemberRequest {
            company_id: id.clone(),
            email: "Bo@Example.se".into(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let after = api
        .list_companies(authed(pb::ListCompaniesRequest {}, &bo))
        .await
        .unwrap()
        .into_inner();
    let members = api
        .list_members(authed(
            pb::ListMembersRequest {
                company_id: id.clone(),
            },
            &bo,
        ))
        .await
        .unwrap()
        .into_inner();
    let unknown = api
        .add_member(authed(
            pb::AddMemberRequest {
                company_id: id,
                email: "nobody@example.se".into(),
            },
            &anna,
        ))
        .await
        .unwrap_err();

    assert!(before.companies.is_empty());
    assert_eq!(after.companies.len(), 1);
    let emails: Vec<_> = members.members.iter().map(|m| m.email.as_str()).collect();
    assert_eq!(emails, ["anna@example.se", "bo@example.se"]);
    assert_eq!(code_of(unknown), (Code::NotFound, "user_not_found".into()));
}

#[tokio::test]
async fn non_members_and_bad_ids_get_company_not_found() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let mut api = server.companies();
    let id = api
        .create_company(authed(create("556016-0680", "Exempel AB"), &anna))
        .await
        .unwrap()
        .into_inner()
        .company_id;
    let not_found = (Code::NotFound, "company_not_found".to_owned());

    for company_id in [
        id.clone(),
        "not-a-uuid".into(),
        uuid::Uuid::new_v4().to_string(),
    ] {
        let get = api
            .get_company(authed(
                pb::GetCompanyRequest {
                    company_id: company_id.clone(),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        let members = api
            .list_members(authed(
                pb::ListMembersRequest {
                    company_id: company_id.clone(),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        // Probing an unknown email must not reveal that it is unknown.
        let add = api
            .add_member(authed(
                pb::AddMemberRequest {
                    company_id,
                    email: "nobody@example.se".into(),
                },
                &bo,
            ))
            .await
            .unwrap_err();
        assert_eq!(code_of(get), not_found);
        assert_eq!(code_of(members), not_found);
        assert_eq!(code_of(add), not_found);
    }
}
