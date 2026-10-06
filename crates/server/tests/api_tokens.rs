mod common;

use common::{TestServer, authed, company, device, in_days};
use doris_proto::auth::v1 as pb;
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
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let created = api
        .create_api_token(authed(
            create(
                "Agent",
                in_days(90),
                vec![grant(&id, &["ledger:write", "ledger:read"])],
            ),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
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
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();
    let created = api
        .create_api_token(authed(
            create("Agent", in_days(1), vec![grant(&id, &["ledger:read"])]),
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
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
        .create_api_token(authed(create("Agent", in_days(30), vec![]), &anna))
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
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let annas = company(&server, &anna, "556016-0680").await;
    let mut api = server.grpc();

    let not_member = api
        .create_api_token(authed(
            create("Agent", in_days(30), vec![grant(&annas, &["ledger:read"])]),
            &bo,
        ))
        .await
        .unwrap_err();
    let unknown_scope = api
        .create_api_token(authed(
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
            .create_api_token(authed(try_day(day.clone()), &anna))
            .await
            .unwrap_err();
        assert_eq!(
            code_of(err),
            (Code::InvalidArgument, "invalid_token_expiry".into()),
            "{day}"
        );
    }
    api.create_api_token(authed(try_day(in_days(366)), &anna))
        .await
        .unwrap();
}

#[tokio::test]
async fn someone_elses_token_cannot_be_seen_or_revoked_but_an_admin_can_revoke_it() {
    let server = TestServer::start().await;
    let anna = server.sign_up(&mut device(), "anna@example.se", None).await;
    let bo = server.invite(&anna, "bo@example.se").await;
    let cecilia = server.invite(&anna, "cecilia@example.se").await;
    let bos_company = company(&server, &bo, "556016-0680").await;
    let mut api = server.grpc();
    let created = api
        .create_api_token(authed(
            create(
                "Bo",
                in_days(30),
                vec![grant(&bos_company, &["ledger:read"])],
            ),
            &bo,
        ))
        .await
        .unwrap()
        .into_inner();
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
