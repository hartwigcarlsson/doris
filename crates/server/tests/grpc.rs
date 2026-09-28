mod common;

use common::{TestServer, authed, device, session_from, set_cookie};
use doris_proto::auth::v1 as pb;
use tonic::{Code, Request};
use webauthn_rs::prelude::CreationChallengeResponse;

#[tokio::test]
async fn status_reports_bootstrap_until_the_first_user_registers() {
    let server = TestServer::start().await;

    let before = server
        .grpc()
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    let session = server.sign_up(&mut device(), "anna@example.se", None).await;
    let anonymous = server
        .grpc()
        .get_status(pb::GetStatusRequest {})
        .await
        .unwrap()
        .into_inner();
    let signed_in = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &session))
        .await
        .unwrap()
        .into_inner();

    assert!(before.bootstrap_required);
    assert_eq!(before.current_user, None);
    assert!(!anonymous.bootstrap_required);
    assert_eq!(anonymous.current_user, None);
    let user = signed_in.current_user.unwrap();
    assert_eq!(user.email, "anna@example.se");
    assert_eq!(user.role(), pb::Role::Admin);
}

#[tokio::test]
async fn the_session_cookie_is_http_only_secure_and_strict() {
    let server = TestServer::start().await;
    let mut grpc = server.grpc();
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;

    let begin = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "anna@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let options = serde_json::from_str(&begin.options_json).unwrap();
    let credential = laptop
        .do_authentication(server.origin.clone(), options)
        .unwrap();
    let response = grpc
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: begin.ceremony_id,
            credential_json: serde_json::to_string(&credential).unwrap(),
        })
        .await
        .unwrap();

    let cookie = set_cookie(response.metadata()).unwrap();
    let attributes: Vec<&str> = cookie.split("; ").skip(1).collect();
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "Secure",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=2592000"
        ]
    );
    assert_eq!(response.into_inner().email, "anna@example.se");
}

#[tokio::test]
async fn logging_out_clears_the_cookie_and_ends_the_session() {
    let server = TestServer::start().await;
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;
    let session = server.log_in(&mut laptop, "anna@example.se").await.unwrap();

    let response = server
        .grpc()
        .logout(authed(pb::LogoutRequest {}, &session))
        .await
        .unwrap();
    let status = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &session))
        .await
        .unwrap()
        .into_inner();

    let cookie = set_cookie(response.metadata()).unwrap();
    assert!(cookie.starts_with("doris_session=;"), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");
    assert_eq!(session_from(response.metadata()), None);
    assert_eq!(status.current_user, None);
}

#[tokio::test]
async fn failed_logins_look_the_same_for_known_and_unknown_emails() {
    let server = TestServer::start().await;
    let mut laptop = device();
    server.sign_up(&mut laptop, "anna@example.se", None).await;
    let mut grpc = server.grpc();

    let real = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "anna@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let fake = grpc
        .begin_login(pb::BeginLoginRequest {
            email: "nobody@example.se".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let options = serde_json::from_str(&real.options_json).unwrap();
    let assertion = serde_json::to_string(
        &laptop
            .do_authentication(server.origin.clone(), options)
            .unwrap(),
    )
    .unwrap();
    let finish = |ceremony_id: &str| pb::FinishLoginRequest {
        ceremony_id: ceremony_id.to_owned(),
        credential_json: assertion.clone(),
    };

    let unknown_email = grpc
        .finish_login(finish(&fake.ceremony_id))
        .await
        .unwrap_err();
    grpc.finish_login(finish(&real.ceremony_id)).await.unwrap();
    let replayed = grpc
        .finish_login(finish(&real.ceremony_id))
        .await
        .unwrap_err();

    for err in [unknown_email, replayed] {
        assert_eq!(
            (err.code(), err.message()),
            (Code::Unauthenticated, "login_failed")
        );
    }
}

#[tokio::test]
async fn protected_calls_need_a_session_and_admin_calls_an_admin() {
    let server = TestServer::start().await;
    let admin = server.sign_up(&mut device(), "anna@example.se", None).await;
    let invite = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "bo@example.se".into(),
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner();
    let member = server
        .sign_up(&mut device(), "bo@example.se", Some(&invite.token))
        .await;

    let anonymous = server
        .grpc()
        .list_passkeys(pb::ListPasskeysRequest {})
        .await
        .unwrap_err();
    let bogus = server
        .grpc()
        .list_passkeys(authed(pb::ListPasskeysRequest {}, "bogus"))
        .await
        .unwrap_err();
    let by_member = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "c@example.se".into(),
            },
            &member,
        ))
        .await
        .unwrap_err();
    let list_by_member = server
        .grpc()
        .list_invitations(authed(pb::ListInvitationsRequest {}, &member))
        .await
        .unwrap_err();

    assert_eq!(
        (anonymous.code(), anonymous.message()),
        (Code::Unauthenticated, "not_signed_in")
    );
    assert_eq!(
        (bogus.code(), bogus.message()),
        (Code::Unauthenticated, "not_signed_in")
    );
    assert_eq!(
        (by_member.code(), by_member.message()),
        (Code::PermissionDenied, "not_admin")
    );
    assert_eq!(list_by_member.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn an_admin_invites_a_member_who_registers_through_the_link() {
    let server = TestServer::start().await;
    let admin = server.sign_up(&mut device(), "anna@example.se", None).await;

    let invite = server
        .grpc()
        .create_invitation(authed(
            pb::CreateInvitationRequest {
                email: "Bo@Example.se".into(),
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner();
    let lookup = server
        .grpc()
        .get_invitation(pb::GetInvitationRequest {
            token: invite.token.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut phone = device();
    let member = server
        .sign_up(&mut phone, "bo@example.se", Some(&invite.token))
        .await;
    let used = server
        .grpc()
        .get_invitation(pb::GetInvitationRequest {
            token: invite.token.clone(),
        })
        .await
        .unwrap_err();
    let listed = server
        .grpc()
        .list_invitations(authed(pb::ListInvitationsRequest {}, &admin))
        .await
        .unwrap()
        .into_inner();
    let status = server
        .grpc()
        .get_status(authed(pb::GetStatusRequest {}, &member))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(lookup.email, "bo@example.se");
    assert!(invite.expires_at.ends_with('Z'), "{}", invite.expires_at);
    assert_eq!(
        (used.code(), used.message()),
        (Code::NotFound, "invitation_not_found")
    );
    assert_eq!(listed.invitations.len(), 1);
    assert!(listed.invitations[0].accepted);
    assert_eq!(status.current_user.unwrap().role(), pb::Role::Member);
    assert!(server.log_in(&mut phone, "bo@example.se").await.is_ok());
}

#[tokio::test]
async fn a_signed_in_user_adds_and_lists_passkeys() {
    let server = TestServer::start().await;
    let session = server.sign_up(&mut device(), "anna@example.se", None).await;
    let mut phone = device();
    let mut grpc = server.grpc();

    let begin = grpc
        .begin_add_passkey(authed(
            pb::BeginAddPasskeyRequest {
                passkey_name: "Telefon".into(),
            },
            &session,
        ))
        .await
        .unwrap()
        .into_inner();
    let options: CreationChallengeResponse = serde_json::from_str(&begin.options_json).unwrap();
    let credential = phone
        .do_registration(server.origin.clone(), options)
        .unwrap();
    grpc.finish_add_passkey(authed(
        pb::FinishAddPasskeyRequest {
            ceremony_id: begin.ceremony_id,
            credential_json: serde_json::to_string(&credential).unwrap(),
        },
        &session,
    ))
    .await
    .unwrap();
    let passkeys = grpc
        .list_passkeys(authed(pb::ListPasskeysRequest {}, &session))
        .await
        .unwrap()
        .into_inner()
        .passkeys;

    let names: Vec<&str> = passkeys.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert!(server.log_in(&mut phone, "anna@example.se").await.is_ok());
}

#[tokio::test]
async fn errors_carry_stable_codes_for_the_frontend() {
    let server = TestServer::start().await;
    let mut grpc = server.grpc();
    let begin = |email: &str, name: &str| pb::BeginRegistrationRequest {
        email: email.into(),
        display_name: name.into(),
        invitation_token: None,
        passkey_name: "Laptop".into(),
    };

    let bad_email = grpc
        .begin_registration(begin("anna", "Anna"))
        .await
        .unwrap_err();
    let bad_name = grpc
        .begin_registration(begin("anna@example.se", " "))
        .await
        .unwrap_err();
    server.sign_up(&mut device(), "anna@example.se", None).await;
    let uninvited = grpc
        .begin_registration(begin("bo@example.se", "Bo"))
        .await
        .unwrap_err();
    let bad_ceremony = grpc
        .finish_login(pb::FinishLoginRequest {
            ceremony_id: "x".into(),
            credential_json: "{}".into(),
        })
        .await
        .unwrap_err();
    let unknown_invite = grpc
        .get_invitation(Request::new(pb::GetInvitationRequest {
            token: "nope".into(),
        }))
        .await
        .unwrap_err();

    assert_eq!(
        (bad_email.code(), bad_email.message()),
        (Code::InvalidArgument, "invalid_email")
    );
    assert_eq!(
        (bad_name.code(), bad_name.message()),
        (Code::InvalidArgument, "invalid_display_name")
    );
    assert_eq!(
        (uninvited.code(), uninvited.message()),
        (Code::InvalidArgument, "invitation_required")
    );
    assert_eq!(
        (bad_ceremony.code(), bad_ceremony.message()),
        (Code::InvalidArgument, "invalid_ceremony")
    );
    assert_eq!(unknown_invite.code(), Code::NotFound);
}
