use doris_identity::domain::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str) -> Passkey {
    Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap()
}

fn cmd(email: &str) -> RegisterUser {
    RegisterUser {
        user_id: Uuid::new_v4(),
        email: Email::parse(email).unwrap(),
        display_name: DisplayName::parse("Anna Andersson").unwrap(),
        passkey: passkey("cred-1"),
    }
}

fn user(role: Role) -> User {
    User {
        id: Uuid::new_v4(),
        email: Email::parse("anna@example.se").unwrap(),
        display_name: DisplayName::parse("Anna").unwrap(),
        role,
        passkeys: vec![passkey("cred-1")],
    }
}

fn invitation(email: &str) -> Invitation {
    Invitation {
        id: Uuid::new_v4(),
        email: Email::parse(email).unwrap(),
        token_hash: "hash".into(),
        created_by: Uuid::new_v4(),
        expires_at: now() + SignedDuration::from_hours(1),
        accepted_by: None,
    }
}

#[test]
fn email_is_trimmed_and_lowercased() {
    let email = Email::parse("  Anna.Andersson@Example.SE ").unwrap();
    assert_eq!(email.as_str(), "anna.andersson@example.se");
}

#[test]
fn malformed_emails_are_rejected() {
    let long = format!("{}@example.se", "a".repeat(250));
    for raw in [
        "",
        "anna",
        "@example.se",
        "anna@",
        "anna@example",
        "a@b@c.se",
        "an na@example.se",
        "anna@.se",
        "anna@example.",
        &long,
    ] {
        assert_eq!(Email::parse(raw), Err(DomainError::InvalidEmail), "{raw:?}");
    }
}

#[test]
fn display_name_must_be_1_to_100_characters() {
    assert_eq!(DisplayName::parse(" Åsa ").unwrap().as_str(), "Åsa");
    assert!(DisplayName::parse(&"å".repeat(100)).is_ok());
    assert_eq!(
        DisplayName::parse("   "),
        Err(DomainError::InvalidDisplayName)
    );
    assert_eq!(
        DisplayName::parse(&"å".repeat(101)),
        Err(DomainError::InvalidDisplayName)
    );
}

#[test]
fn passkey_name_must_be_1_to_64_characters() {
    assert!(Passkey::new("c".into(), &"x".repeat(64), json!({})).is_ok());
    assert_eq!(
        Passkey::new("c".into(), " ", json!({})),
        Err(DomainError::InvalidPasskeyName)
    );
    assert_eq!(
        Passkey::new("c".into(), &"x".repeat(65), json!({})),
        Err(DomainError::InvalidPasskeyName)
    );
}

#[test]
fn bootstrap_registration_creates_an_admin_with_a_passkey() {
    let cmd = cmd("anna@example.se");
    let user_id = cmd.user_id;

    let (events, accepted) = register_user(Admission::Bootstrap, cmd, now()).unwrap();

    let user = User::from_events(&events).unwrap();
    assert_eq!(user.id, user_id);
    assert_eq!(user.role, Role::Admin);
    assert_eq!(user.passkeys, [passkey("cred-1")]);
    assert_eq!(accepted, None);
}

#[test]
fn registration_without_invitation_after_bootstrap_is_rejected() {
    let err = register_user(Admission::Uninvited, cmd("anna@example.se"), now()).unwrap_err();
    assert_eq!(err, DomainError::InvitationRequired);
}

#[test]
fn invited_registration_creates_a_member_and_accepts_the_invitation() {
    let invitation = invitation("bo@example.se");
    let cmd = cmd("bo@example.se");
    let user_id = cmd.user_id;

    let (events, accepted) = register_user(Admission::Invited(&invitation), cmd, now()).unwrap();

    assert_eq!(User::from_events(&events).unwrap().role, Role::Member);
    assert!(matches!(
        &events[0],
        UserEvent::UserRegistered { invitation_id: Some(id), .. } if *id == invitation.id
    ));
    assert_eq!(
        accepted,
        Some(InvitationEvent::InvitationAccepted { user_id })
    );
}

#[test]
fn expired_used_or_mismatched_invitations_are_rejected() {
    let mut expired = invitation("bo@example.se");
    expired.expires_at = now();
    let mut used = invitation("bo@example.se");
    used.accepted_by = Some(Uuid::new_v4());
    let other = invitation("cecilia@example.se");

    let register = |inv: &Invitation| {
        register_user(Admission::Invited(inv), cmd("bo@example.se"), now()).unwrap_err()
    };

    assert_eq!(register(&expired), DomainError::InvitationExpired);
    assert_eq!(register(&used), DomainError::InvitationAlreadyUsed);
    assert_eq!(register(&other), DomainError::InvitationEmailMismatch);
}

#[test]
fn adding_a_passkey_the_user_already_has_is_rejected() {
    let user = user(Role::Member);
    assert_eq!(
        add_passkey(&user, passkey("cred-1")),
        Err(DomainError::DuplicatePasskey)
    );

    let events = add_passkey(&user, passkey("cred-2")).unwrap();
    let mut all = vec![UserEvent::UserRegistered {
        user_id: user.id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        role: user.role,
        invitation_id: None,
    }];
    all.extend(events);
    assert_eq!(
        User::from_events(&all).unwrap().passkeys,
        [passkey("cred-2")]
    );
}

#[test]
fn passkey_use_replaces_the_stored_credential() {
    let user = user(Role::Member);
    assert_eq!(
        record_passkey_use(&user, "nope", json!({})),
        Err(DomainError::UnknownPasskey)
    );

    let events = record_passkey_use(&user, "cred-1", json!({ "counter": 5 })).unwrap();

    let registered = UserEvent::UserRegistered {
        user_id: user.id,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        role: user.role,
        invitation_id: None,
    };
    let added = UserEvent::PasskeyAdded {
        credential_id: "cred-1".into(),
        name: "Laptop".into(),
        passkey: json!({}),
    };
    let state = User::from_events([&registered, &added].into_iter().chain(&events)).unwrap();
    assert_eq!(state.passkeys[0].passkey, json!({ "counter": 5 }));
}

#[test]
fn only_admins_create_invitations_valid_for_seven_days() {
    let email = Email::parse("bo@example.se").unwrap();
    let id = Uuid::new_v4();

    assert_eq!(
        create_invitation(&user(Role::Member), id, email.clone(), "h".into(), now()),
        Err(DomainError::NotAdmin)
    );

    let admin = user(Role::Admin);
    let event = create_invitation(&admin, id, email, "h".into(), now()).unwrap();
    let invitation = Invitation::from_events([&event]).unwrap();
    assert_eq!(invitation.created_by, admin.id);
    assert_eq!(
        invitation.expires_at,
        now() + SignedDuration::from_hours(24 * 7)
    );
}
