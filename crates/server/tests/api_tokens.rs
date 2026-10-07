mod common;

use common::{TestServer, api_token, authed, bearer, company, device, in_days};
use doris_proto::auth::v1 as pb;
use doris_proto::company::v1 as cpb;
use doris_proto::invoicing::v1 as ipb;
use doris_proto::ledger::v1 as lpb;
use doris_proto::payroll::v1 as ppb;
use doris_proto::vat::v1 as vpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

fn create(
    name: &str,
    expires_on: String,
    grants: Vec<pb::TokenGrant>,
) -> pb::CreateApiTokenRequest {
    pb::CreateApiTokenRequest {
        name: name.into(),
        expires_on,
        grants,
    }
}

fn grant(company: &str, scopes: &[&str]) -> pb::TokenGrant {
    pb::TokenGrant {
        company_id: company.into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn a_user_creates_lists_and_revokes_a_token() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let created = server
        .create_token(
            &anna,
            &mut annas,
            create(
                "Agent",
                in_days(90),
                vec![grant(&id, &["ledger:write", "ledger:read"])],
            ),
        )
        .await
        .unwrap();
    let listed = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;

    assert!(created.secret.starts_with("doris_"));
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.token_id);
    assert_eq!(listed[0].name, "Agent");
    assert_eq!(
        listed[0].grants,
        vec![grant(&id, &["ledger:read", "ledger:write"])]
    );
    assert_eq!(listed[0].revoked_at, None);

    api.revoke_api_token(authed(
        pb::RevokeApiTokenRequest {
            token_id: created.token_id.clone(),
        },
        &anna,
    ))
    .await
    .unwrap();
    let listed = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert!(listed[0].revoked_at.is_some());
}

#[tokio::test]
async fn revoking_twice_is_fine() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();
    let created = server
        .create_token(
            &anna,
            &mut annas,
            create("Agent", in_days(1), vec![grant(&id, &["ledger:read"])]),
        )
        .await
        .unwrap();
    let revoke = || pb::RevokeApiTokenRequest {
        token_id: created.token_id.clone(),
    };

    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
}

#[tokio::test]
async fn a_token_without_grants_is_refused() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;

    let err = server
        .grpc()
        .begin_create_api_token(authed(create("Agent", in_days(30), vec![]), &anna))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(err),
        (Code::InvalidArgument, "invalid_token_grants".into())
    );
}

#[tokio::test]
async fn a_token_is_only_for_the_users_own_companies_and_known_scopes() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let annas = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let not_member = api
        .begin_create_api_token(authed(
            create("Agent", in_days(30), vec![grant(&annas, &["ledger:read"])]),
            &bo,
        ))
        .await
        .unwrap_err();
    let unknown_scope = api
        .begin_create_api_token(authed(
            create("Agent", in_days(30), vec![grant(&annas, &["ledger:admin"])]),
            &anna,
        ))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(not_member),
        (Code::NotFound, "company_not_found".into())
    );
    assert_eq!(
        code_of(unknown_scope),
        (Code::InvalidArgument, "invalid_token_grants".into())
    );
}

#[tokio::test]
async fn the_last_day_is_today_at_the_earliest_and_a_year_off_at_most() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();
    let try_day = |day: String| create("Agent", day, vec![grant(&id, &["ledger:read"])]);

    for day in [in_days(-1), in_days(367), "i morgon".to_owned()] {
        let err = api
            .begin_create_api_token(authed(try_day(day.clone()), &anna))
            .await
            .unwrap_err();
        assert_eq!(
            code_of(err),
            (Code::InvalidArgument, "invalid_token_expiry".into()),
            "{day}"
        );
    }
    api.begin_create_api_token(authed(try_day(in_days(366)), &anna))
        .await
        .unwrap();
}

#[tokio::test]
async fn someone_elses_token_cannot_be_seen_or_revoked_but_an_admin_can_revoke_it() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let mut bos = device();
    let bo = server
        .invite_with(&anna, &mut annas, "bo@example.se", &mut bos)
        .await;
    let cecilia = server.invite(&anna, &mut annas, "cecilia@example.se").await;
    let bos_company = company(&server, &bo, "556016-0680").await;
    let mut api = server.grpc();
    let created = server
        .create_token(
            &bo,
            &mut bos,
            create(
                "Bo",
                in_days(30),
                vec![grant(&bos_company, &["ledger:read"])],
            ),
        )
        .await
        .unwrap();
    let revoke = || pb::RevokeApiTokenRequest {
        token_id: created.token_id.clone(),
    };

    let by_cecilia = api
        .revoke_api_token(authed(revoke(), &cecilia))
        .await
        .unwrap_err();
    let unknown = api
        .revoke_api_token(authed(
            pb::RevokeApiTokenRequest {
                token_id: "nej".into(),
            },
            &bo,
        ))
        .await
        .unwrap_err();
    let cecilias_list = api
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &cecilia))
        .await
        .unwrap()
        .into_inner()
        .tokens;

    assert_eq!(
        code_of(by_cecilia),
        (Code::NotFound, "api_token_not_found".into())
    );
    assert_eq!(
        code_of(unknown),
        (Code::NotFound, "api_token_not_found".into())
    );
    assert!(cecilias_list.is_empty());
    api.revoke_api_token(authed(revoke(), &anna)).await.unwrap();
}

fn sale(company: &str) -> lpb::RecordVoucherRequest {
    lpb::RecordVoucherRequest {
        company_id: company.into(),
        date: "2026-01-15".into(),
        text: "Försäljning".into(),
        lines: vec![
            lpb::VoucherLine {
                account: 1930,
                debit: 100,
                credit: 0,
            },
            lpb::VoucherLine {
                account: 3001,
                debit: 0,
                credit: 100,
            },
        ],
        attachments: vec![],
        dry_run: false,
    }
}

fn vouchers(company: &str) -> lpb::ListVouchersRequest {
    lpb::ListVouchersRequest {
        company_id: company.into(),
        fiscal_year_start: "2026-01-01".into(),
    }
}

#[tokio::test]
async fn a_read_token_lists_but_does_not_book_and_a_write_token_books_with_its_id_recorded() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let reader = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let writer = api_token(
        &server,
        &anna,
        &mut annas,
        &[(&id, &["ledger:read", "ledger:write"])],
    )
    .await;
    let mut ledger = server.ledger();

    ledger
        .list_vouchers(bearer(vouchers(&id), &reader))
        .await
        .unwrap();
    let refused = ledger
        .record_voucher(bearer(sale(&id), &reader))
        .await
        .unwrap_err();
    ledger
        .record_voucher(bearer(sale(&id), &writer))
        .await
        .unwrap();

    assert_eq!(
        code_of(refused),
        (Code::PermissionDenied, "missing_scope".into())
    );
    let token_id = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens
        .into_iter()
        .find(|t| t.grants[0].scopes.len() == 2)
        .unwrap()
        .id;
    let metadata: String = sqlx::query_scalar(
        "SELECT metadata FROM events WHERE event_type = 'VoucherRecorded'
         ORDER BY global_position DESC LIMIT 1",
    )
    .fetch_one(&server.pool)
    .await
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["via_token"], token_id.as_str());
}

#[tokio::test]
async fn scopes_are_per_company() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let a = company(&server, &anna, "556016-0680").await;
    let b = company(&server, &anna, "556036-0793").await;
    let token = api_token(
        &server,
        &anna,
        &mut annas,
        &[(&a, &["ledger:write"]), (&b, &["ledger:read"])],
    )
    .await;
    let mut ledger = server.ledger();

    ledger
        .record_voucher(bearer(sale(&a), &token))
        .await
        .unwrap();
    let in_b = ledger
        .record_voucher(bearer(sale(&b), &token))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(in_b),
        (Code::PermissionDenied, "missing_scope".into())
    );
}

#[tokio::test]
async fn a_company_outside_the_grants_does_not_exist_for_the_token() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let granted = company(&server, &anna, "556016-0680").await;
    let other = company(&server, &anna, "556036-0793").await;
    let token = api_token(&server, &anna, &mut annas, &[(&granted, &["ledger:read"])]).await;

    let err = server
        .ledger()
        .list_vouchers(bearer(vouchers(&other), &token))
        .await
        .unwrap_err();
    let listed = server
        .companies()
        .list_companies(bearer(cpb::ListCompaniesRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .companies;

    assert_eq!(code_of(err), (Code::NotFound, "company_not_found".into()));
    assert_eq!(
        listed.into_iter().map(|c| c.id).collect::<Vec<_>>(),
        [granted]
    );
}

#[tokio::test]
async fn expired_revoked_and_malformed_tokens_are_not_signed_in_even_with_a_cookie() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let me = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .current_user
        .unwrap()
        .id;
    let then = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(48);
    let (_, expired) = doris_identity::create_api_token(
        &server.pool,
        me.parse().unwrap(),
        "Gammal",
        then + jiff::SignedDuration::from_hours(24),
        vec![doris_identity::domain::Grant {
            company_id: id.parse().unwrap(),
            scopes: vec![doris_identity::domain::Scope::LedgerRead],
        }],
        then,
    )
    .await
    .unwrap();
    let revoked = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let revoked_id = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens[0]
        .id
        .clone();
    server
        .grpc()
        .revoke_api_token(authed(
            pb::RevokeApiTokenRequest {
                token_id: revoked_id,
            },
            &anna,
        ))
        .await
        .unwrap();

    for secret in [expired.as_str(), revoked.as_str(), "doris_nope", "nope"] {
        let mut request = bearer(vouchers(&id), secret);
        request.metadata_mut().insert(
            "cookie",
            format!("{}={anna}", doris_server::SESSION_COOKIE)
                .parse()
                .unwrap(),
        );
        let err = server.ledger().list_vouchers(request).await.unwrap_err();
        assert_eq!(
            code_of(err),
            (Code::Unauthenticated, "not_signed_in".into()),
            "{secret}"
        );
    }
}

#[tokio::test]
async fn a_token_cannot_manage_tokens_invite_or_create_companies_but_knows_its_owner() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(
        &server,
        &anna,
        &mut annas,
        &[(&id, &["ledger:read", "company:read"])],
    )
    .await;
    let mut auth = server.grpc();

    let errors = [
        auth.begin_create_api_token(bearer(
            create("Ny", in_days(1), vec![grant(&id, &["ledger:read"])]),
            &token,
        ))
        .await
        .unwrap_err(),
        auth.begin_create_invitation(bearer(
            pb::CreateInvitationRequest {
                email: "bo@example.se".into(),
            },
            &token,
        ))
        .await
        .unwrap_err(),
        server
            .companies()
            .create_company(bearer(
                cpb::CreateCompanyRequest {
                    org_nr: "556036-0793".into(),
                    name: "Nytt AB".into(),
                    legal_form: cpb::LegalForm::Aktiebolag as i32,
                    address: None,
                    fiscal_year_start: "2026-01-01".into(),
                    fiscal_year_end: "2026-12-31".into(),
                    accounting_method: cpb::AccountingMethod::Invoice as i32,
                },
                &token,
            ))
            .await
            .unwrap_err(),
    ];
    let status = auth
        .get_status(bearer(pb::GetStatusRequest {}, &token))
        .await
        .unwrap()
        .into_inner();
    let company = server
        .companies()
        .get_company(bearer(
            cpb::GetCompanyRequest {
                company_id: id.clone(),
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();

    for err in errors {
        assert_eq!(
            code_of(err),
            (Code::PermissionDenied, "token_not_allowed".into())
        );
    }
    assert_eq!(status.current_user.unwrap().email, "anna@example.se");
    assert_eq!(company.id, id);
}

#[tokio::test]
async fn the_bearer_scheme_is_case_insensitive_and_spaces_are_ignored() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;

    for header in [format!("bearer {token}"), format!("BEARER  {token} ")] {
        let mut request = tonic::Request::new(vouchers(&id));
        request
            .metadata_mut()
            .insert("authorization", header.parse().unwrap());
        server.ledger().list_vouchers(request).await.unwrap();
    }
}

#[tokio::test]
async fn each_service_checks_the_area() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let ledger_only = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let all = api_token(
        &server,
        &anna,
        &mut annas,
        &[(&id, &["payroll:read", "invoicing:read", "vat:read"])],
    )
    .await;
    let employees = || ppb::ListEmployeesRequest {
        company_id: id.clone(),
    };
    let customers = || ipb::ListCustomersRequest {
        company_id: id.clone(),
    };
    let returns = || vpb::ListVatReturnsRequest {
        company_id: id.clone(),
        fiscal_year_start: "2026-01-01".into(),
    };

    let refused = [
        server
            .payroll()
            .list_employees(bearer(employees(), &ledger_only))
            .await
            .unwrap_err(),
        server
            .invoicing()
            .list_customers(bearer(customers(), &ledger_only))
            .await
            .unwrap_err(),
        server
            .vat()
            .list_vat_returns(bearer(returns(), &ledger_only))
            .await
            .unwrap_err(),
    ];
    for err in refused {
        assert_eq!(
            code_of(err),
            (Code::PermissionDenied, "missing_scope".into())
        );
    }
    server
        .payroll()
        .list_employees(bearer(employees(), &all))
        .await
        .unwrap();
    server
        .invoicing()
        .list_customers(bearer(customers(), &all))
        .await
        .unwrap();
    server
        .vat()
        .list_vat_returns(bearer(returns(), &all))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_token_sends_large_underlag_through_the_gate() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:write"])]).await;
    let pdf = |size: usize| {
        let mut data = b"%PDF-1.7\n".to_vec();
        data.resize(size, b'x');
        data
    };
    let mut request = sale(&id);
    request.attachments = vec![
        lpb::NewAttachment {
            file_name: "a.pdf".into(),
            data: pdf(10 << 20),
        },
        lpb::NewAttachment {
            file_name: "b.pdf".into(),
            data: pdf((10 << 20) - 1),
        },
    ];

    server
        .ledger()
        .record_voucher(bearer(request, &token))
        .await
        .unwrap();
}

#[tokio::test]
async fn another_authorization_scheme_does_not_hide_the_session() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut request = authed(pb::GetStatusRequest {}, &anna);
    request
        .metadata_mut()
        .insert("authorization", "Basic eDp5".parse().unwrap());

    let status = server
        .grpc()
        .get_status(request)
        .await
        .unwrap()
        .into_inner();

    assert!(status.current_user.is_some());
}

#[tokio::test]
async fn a_call_with_a_token_records_its_last_use() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let token = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let last_used = || async {
        server
            .grpc()
            .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
            .await
            .unwrap()
            .into_inner()
            .tokens[0]
            .last_used_at
            .clone()
    };
    assert_eq!(last_used().await, None);

    server
        .ledger()
        .list_vouchers(bearer(vouchers(&id), &token))
        .await
        .unwrap();

    assert!(last_used().await.is_some());
}

fn change_of(token_id: &str, company: &str, scopes: &[&str]) -> pb::ChangeApiTokenRequest {
    pb::ChangeApiTokenRequest {
        token_id: token_id.into(),
        name: "Agent".into(),
        expires_on: in_days(30),
        grants: vec![grant(company, scopes)],
    }
}

async fn only_token_id(server: &TestServer, session: &str) -> String {
    server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, session))
        .await
        .unwrap()
        .into_inner()
        .tokens[0]
        .id
        .clone()
}

#[tokio::test]
async fn a_token_is_changed_with_a_passkey_and_its_secret_keeps_working() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let token_id = only_token_id(&server, &anna).await;
    let mut ledger = server.ledger();
    let before = ledger
        .record_voucher(bearer(sale(&id), &secret))
        .await
        .unwrap_err();

    server
        .change_token(
            &anna,
            &mut annas,
            change_of(&token_id, &id, &["ledger:read", "ledger:write"]),
        )
        .await
        .unwrap();
    ledger
        .record_voucher(bearer(sale(&id), &secret))
        .await
        .unwrap();
    server
        .change_token(
            &anna,
            &mut annas,
            change_of(&token_id, &id, &["company:read"]),
        )
        .await
        .unwrap();
    let after = ledger
        .list_vouchers(bearer(vouchers(&id), &secret))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(before),
        (Code::PermissionDenied, "missing_scope".into())
    );
    assert_eq!(
        code_of(after),
        (Code::PermissionDenied, "missing_scope".into())
    );
    let listed = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert_eq!(listed.len(), 1, "changed, not replaced");
    assert_eq!(listed[0].grants, vec![grant(&id, &["company:read"])]);
}

#[tokio::test]
async fn each_ceremony_is_finished_once_by_the_user_who_began_it() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_create_api_token(authed(
            create("Agent", in_days(30), vec![grant(&id, &["ledger:read"])]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishConfirmationRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id.clone(),
    };

    let by_bo = auth
        .finish_create_api_token(authed(finish.clone(), &bo))
        .await
        .unwrap_err();
    let again = auth
        .finish_create_api_token(authed(finish, &anna))
        .await
        .unwrap_err();

    // A create ceremony finished as a change.
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let begin = auth
        .begin_create_api_token(authed(
            create("Agent", in_days(30), vec![grant(&id, &["ledger:read"])]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishConfirmationRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };
    let wrong_kind = auth
        .finish_change_api_token(authed(finish, &anna))
        .await
        .unwrap_err();

    for err in [by_bo, again, wrong_kind] {
        assert_eq!(
            code_of(err),
            (Code::FailedPrecondition, "ceremony_expired".into())
        );
    }
    assert!(!secret.is_empty());
}

#[tokio::test]
async fn a_change_is_refused_at_begin_before_any_passkey() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let mut bos = device();
    let bo = server
        .invite_with(&anna, &mut annas, "bo@example.se", &mut bos)
        .await;
    let annas_company = company(&server, &anna, "556016-0680").await;
    let bos_company = company(&server, &bo, "556036-0793").await;
    api_token(
        &server,
        &anna,
        &mut annas,
        &[(&annas_company, &["ledger:read"])],
    )
    .await;
    let annas_token = only_token_id(&server, &anna).await;
    api_token(&server, &bo, &mut bos, &[(&bos_company, &["ledger:read"])]).await;
    let bos_token = only_token_id(&server, &bo).await;
    let begin = |request: pb::ChangeApiTokenRequest| {
        let mut auth = server.grpc();
        let anna = anna.clone();
        async move {
            auth.begin_change_api_token(authed(request, &anna))
                .await
                .unwrap_err()
        }
    };

    let mut empty_name = change_of(&annas_token, &annas_company, &["ledger:read"]);
    empty_name.name = " ".into();
    let errors = [
        (
            begin(empty_name).await,
            (Code::InvalidArgument, "invalid_token_name"),
        ),
        (
            begin(change_of(&annas_token, &annas_company, &[])).await,
            (Code::InvalidArgument, "invalid_token_grants"),
        ),
        (
            begin(change_of(&annas_token, &bos_company, &["ledger:read"])).await,
            (Code::NotFound, "company_not_found"),
        ),
        (
            begin(change_of(&bos_token, &annas_company, &["ledger:read"])).await,
            (Code::NotFound, "api_token_not_found"),
        ),
        (
            begin(change_of("nej", &annas_company, &["ledger:read"])).await,
            (Code::NotFound, "api_token_not_found"),
        ),
    ];

    for (err, (code, message)) in errors {
        assert_eq!(code_of(err), (code, message.to_owned()));
    }
    let ceremonies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(ceremonies, 0);
}

#[tokio::test]
async fn a_token_revoked_during_the_ceremony_is_not_changed() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    api_token(&server, &anna, &mut annas, &[(&id, &["ledger:read"])]).await;
    let token_id = only_token_id(&server, &anna).await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_change_api_token(authed(change_of(&token_id, &id, &["ledger:write"]), &anna))
        .await
        .unwrap()
        .into_inner();
    auth.revoke_api_token(authed(pb::RevokeApiTokenRequest { token_id }, &anna))
        .await
        .unwrap();
    let finish = pb::FinishConfirmationRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };

    let err = auth
        .finish_change_api_token(authed(finish, &anna))
        .await
        .unwrap_err();

    assert_eq!(
        code_of(err),
        (Code::FailedPrecondition, "api_token_revoked".into())
    );
}

#[tokio::test]
async fn an_expired_token_gets_a_new_last_day() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let me: uuid::Uuid = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .current_user
        .unwrap()
        .id
        .parse()
        .unwrap();
    let then = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(48);
    let (token_id, secret) = doris_identity::create_api_token(
        &server.pool,
        me,
        "Gammal",
        then + jiff::SignedDuration::from_hours(24),
        vec![doris_identity::domain::Grant {
            company_id: id.parse().unwrap(),
            scopes: vec![doris_identity::domain::Scope::LedgerRead],
        }],
        then,
    )
    .await
    .unwrap();
    let mut ledger = server.ledger();
    let before = ledger
        .list_vouchers(bearer(vouchers(&id), &secret))
        .await
        .unwrap_err();

    server
        .change_token(
            &anna,
            &mut annas,
            change_of(&token_id.to_string(), &id, &["ledger:read"]),
        )
        .await
        .unwrap();

    assert_eq!(
        code_of(before),
        (Code::Unauthenticated, "not_signed_in".into())
    );
    ledger
        .list_vouchers(bearer(vouchers(&id), &secret))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_dry_run_with_a_token_does_not_count_as_use() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let secret = api_token(&server, &anna, &mut annas, &[(&id, &["ledger:write"])]).await;
    let mut request = sale(&id);
    request.dry_run = true;

    server
        .ledger()
        .record_voucher(bearer(request, &secret))
        .await
        .unwrap();

    let listed = server
        .grpc()
        .list_api_tokens(authed(pb::ListApiTokensRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .tokens;
    assert_eq!(listed[0].last_used_at, None);
}
