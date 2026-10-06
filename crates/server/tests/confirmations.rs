mod common;

use common::{TestServer, authed, company, device};
use doris_proto::auth::v1 as pb;
use doris_proto::company::v1 as cpb;
use tonic::Code;

fn code_of(err: tonic::Status) -> (Code, String) {
    (err.code(), err.message().to_owned())
}

async fn ceremonies(server: &TestServer) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&server.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_invitation_is_refused_at_begin_before_any_passkey() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let bo = server.invite(&anna, &mut annas, "bo@example.se").await;
    let mut auth = server.grpc();

    let mut errors = vec![];
    for (email, session, expected) in [
        (
            "cecilia@example.se",
            &bo,
            (Code::PermissionDenied, "not_admin"),
        ),
        (
            "inte en adress",
            &anna,
            (Code::InvalidArgument, "invalid_email"),
        ),
        (
            "bo@example.se",
            &anna,
            (Code::AlreadyExists, "already_exists"),
        ),
    ] {
        let err = auth
            .begin_create_invitation(authed(
                pb::CreateInvitationRequest {
                    email: email.into(),
                },
                session,
            ))
            .await
            .unwrap_err();
        errors.push((code_of(err), (expected.0, expected.1.to_owned())));
    }

    for (got, expected) in errors {
        assert_eq!(got, expected);
    }
    assert_eq!(ceremonies(&server).await, 0);
}

#[tokio::test]
async fn a_confirmation_of_one_kind_cannot_finish_another() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let id = company(&server, &anna, "556016-0680").await;
    let mut auth = server.grpc();
    let mut companies = server.companies();

    // An invitation ceremony finished as a new member…
    let begin = auth
        .begin_create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "bo@example.se".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let as_member = companies
        .finish_add_member(authed(
            cpb::FinishAddMemberRequest {
                credential_json: server.confirm(&mut annas, &begin),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    // …a member ceremony finished as an invitation…
    server.invite(&anna, &mut annas, "bo@example.se").await;
    let begin = companies
        .begin_add_member(authed(
            cpb::AddMemberRequest {
                company_id: id.clone(),
                email: "bo@example.se".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let as_invitation = auth
        .finish_create_invitation(authed(
            pb::FinishConfirmationRequest {
                credential_json: server.confirm_options(&mut annas, &begin.options_json),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    // …and a token ceremony finished as an invitation.
    let begin = auth
        .begin_create_api_token(authed(
            pb::CreateApiTokenRequest {
                name: "Agent".into(),
                expires_on: common::in_days(30),
                grants: vec![pb::TokenGrant {
                    company_id: id.clone(),
                    scopes: vec!["ledger:read".into()],
                }],
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let token_finished_as_invitation = auth
        .finish_create_invitation(authed(
            pb::FinishConfirmationRequest {
                credential_json: server.confirm(&mut annas, &begin),
                ceremony_id: begin.ceremony_id,
            },
            &anna,
        ))
        .await
        .unwrap_err();

    for err in [as_member, as_invitation, token_finished_as_invitation] {
        assert_eq!(
            code_of(err),
            (Code::FailedPrecondition, "ceremony_expired".into())
        );
    }
    let members = companies
        .list_members(authed(cpb::ListMembersRequest { company_id: id }, &anna))
        .await
        .unwrap()
        .into_inner()
        .members;
    assert_eq!(members.len(), 1, "nobody was added");
}

#[tokio::test]
async fn an_invitation_ceremony_is_finished_once() {
    let server = TestServer::start().await;
    let mut annas = device();
    let anna = server.sign_up(&mut annas, "anna@example.se", None).await;
    let mut auth = server.grpc();
    let begin = auth
        .begin_create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "bo@example.se".into(),
            },
            &anna,
        ))
        .await
        .unwrap()
        .into_inner();
    let finish = pb::FinishConfirmationRequest {
        credential_json: server.confirm(&mut annas, &begin),
        ceremony_id: begin.ceremony_id,
    };

    let first = auth
        .finish_create_invitation(authed(finish.clone(), &anna))
        .await
        .unwrap()
        .into_inner();
    let again = auth
        .finish_create_invitation(authed(finish, &anna))
        .await
        .unwrap_err();

    assert!(!first.token.is_empty());
    assert_eq!(
        code_of(again),
        (Code::FailedPrecondition, "ceremony_expired".into())
    );
    let listed = auth
        .list_invitations(authed(pb::ListInvitationsRequest {}, &anna))
        .await
        .unwrap()
        .into_inner()
        .invitations;
    assert_eq!(listed.len(), 1);
}
