use doris_identity::domain::*;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-10-06T10:00:00Z".parse().unwrap()
}

fn user(role: Role) -> User {
    User {
        id: Uuid::new_v4(),
        email: Email::parse("anna@example.se").unwrap(),
        display_name: DisplayName::parse("Anna").unwrap(),
        role,
        passkeys: vec![Passkey::new("c1".into(), "Laptop", json!({})).unwrap()],
    }
}

fn grant(company_id: Uuid, scopes: &[Scope]) -> Grant {
    Grant {
        company_id,
        scopes: scopes.to_vec(),
    }
}

fn cmd(name: &str, expires_in: SignedDuration, grants: Vec<Grant>) -> NewApiToken {
    NewApiToken {
        token_id: Uuid::new_v4(),
        name: name.into(),
        expires_at: now() + expires_in,
        grants,
    }
}

const DAY: SignedDuration = SignedDuration::from_hours(24);

#[test]
fn scopes_are_written_as_area_and_level() {
    assert_eq!(Scope::LedgerWrite.as_str(), "ledger:write");
    assert_eq!(Scope::parse("payroll:read"), Some(Scope::PayrollRead));
    assert_eq!(Scope::parse("ledger:admin"), None);
    for scope in Scope::ALL {
        assert_eq!(Scope::parse(scope.as_str()), Some(scope));
        assert_eq!(serde_json::to_value(scope).unwrap(), json!(scope.as_str()));
    }
}

#[test]
fn an_owner_creates_a_token_with_sorted_unique_scopes() {
    let anna = user(Role::Member);
    let company = Uuid::new_v4();
    let cmd = cmd(
        "  Agent  ",
        30 * DAY,
        vec![grant(
            company,
            &[Scope::LedgerWrite, Scope::LedgerRead, Scope::LedgerWrite],
        )],
    );
    let token_id = cmd.token_id;

    let event = create_api_token(&anna, cmd, "hash".into(), now()).unwrap();

    assert_eq!(
        event,
        ApiTokenEvent::ApiTokenCreated {
            token_id,
            user_id: anna.id,
            name: "Agent".into(),
            token_hash: "hash".into(),
            expires_at: now() + 30 * DAY,
            grants: vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])],
        }
    );
}

#[test]
fn a_token_needs_a_name_of_1_to_100_characters() {
    let anna = user(Role::Member);
    let grants = || vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])];
    for name in ["   ", &"å".repeat(101)] {
        assert_eq!(
            create_api_token(&anna, cmd(name, DAY, grants()), "h".into(), now()),
            Err(DomainError::InvalidTokenName),
            "{name:?}"
        );
    }
    assert!(
        create_api_token(
            &anna,
            cmd(&"å".repeat(100), DAY, grants()),
            "h".into(),
            now()
        )
        .is_ok()
    );
}

#[test]
fn a_token_expires_after_now_and_within_368_days() {
    let anna = user(Role::Member);
    let grants = || vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])];
    for expires_in in [
        SignedDuration::ZERO,
        -DAY,
        368 * DAY + SignedDuration::from_secs(1),
    ] {
        assert_eq!(
            create_api_token(&anna, cmd("Agent", expires_in, grants()), "h".into(), now()),
            Err(DomainError::InvalidTokenExpiry),
            "{expires_in:?}"
        );
    }
    assert!(create_api_token(&anna, cmd("Agent", 368 * DAY, grants()), "h".into(), now()).is_ok());
}

#[test]
fn grants_need_a_company_once_and_a_scope_each() {
    let anna = user(Role::Member);
    let company = Uuid::new_v4();
    for grants in [
        vec![],
        vec![grant(company, &[])],
        vec![
            grant(company, &[Scope::LedgerRead]),
            grant(company, &[Scope::VatRead]),
        ],
    ] {
        assert_eq!(
            create_api_token(&anna, cmd("Agent", DAY, grants.clone()), "h".into(), now()),
            Err(DomainError::InvalidTokenGrants),
            "{grants:?}"
        );
    }
}

fn token_of(owner: &User) -> ApiToken {
    let event = create_api_token(
        owner,
        cmd(
            "Agent",
            DAY,
            vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])],
        ),
        "h".into(),
        now(),
    )
    .unwrap();
    ApiToken::from_events(&[event]).unwrap()
}

fn change(name: &str, expires_in: SignedDuration, grants: Vec<Grant>) -> TokenChange {
    TokenChange {
        name: name.into(),
        expires_at: now() + expires_in,
        grants,
    }
}

#[test]
fn the_owner_changes_name_expiry_and_grants() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let company = Uuid::new_v4();

    let events = change_api_token(
        &token,
        &anna,
        change(
            " CLI ",
            90 * DAY,
            vec![grant(company, &[Scope::LedgerWrite, Scope::LedgerRead])],
        ),
        now(),
    )
    .unwrap();

    assert_eq!(
        events,
        vec![ApiTokenEvent::ApiTokenChanged {
            name: "CLI".into(),
            expires_at: now() + 90 * DAY,
            grants: vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])],
        }]
    );
    let changed = ApiToken::from_events(
        [
            ApiTokenEvent::ApiTokenCreated {
                token_id: token.id,
                user_id: anna.id,
                name: token.name.clone(),
                token_hash: "h".into(),
                expires_at: token.expires_at,
                grants: token.grants.clone(),
            },
            events[0].clone(),
        ]
        .iter(),
    )
    .unwrap();
    assert_eq!(changed.name, "CLI");
    assert_eq!(changed.expires_at, now() + 90 * DAY);
    assert_eq!(
        changed.grants,
        vec![grant(company, &[Scope::LedgerRead, Scope::LedgerWrite])]
    );
    assert!(!changed.revoked);
}

#[test]
fn a_change_is_validated_like_a_new_token() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let company = Uuid::new_v4();
    for (bad, expected) in [
        (
            change("  ", DAY, vec![grant(company, &[Scope::LedgerRead])]),
            DomainError::InvalidTokenName,
        ),
        (
            change("CLI", -DAY, vec![grant(company, &[Scope::LedgerRead])]),
            DomainError::InvalidTokenExpiry,
        ),
        (
            change("CLI", 369 * DAY, vec![grant(company, &[Scope::LedgerRead])]),
            DomainError::InvalidTokenExpiry,
        ),
        (change("CLI", DAY, vec![]), DomainError::InvalidTokenGrants),
        (
            change("CLI", DAY, vec![grant(company, &[])]),
            DomainError::InvalidTokenGrants,
        ),
    ] {
        assert_eq!(
            change_api_token(&token, &anna, bad.clone(), now()),
            Err(expected),
            "{bad:?}"
        );
    }
}

#[test]
fn only_the_owner_changes_a_token_not_even_an_admin() {
    let token = token_of(&user(Role::Member));
    let new = change(
        "CLI",
        DAY,
        vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])],
    );
    for actor in [user(Role::Member), user(Role::Admin)] {
        assert_eq!(
            change_api_token(&token, &actor, new.clone(), now()),
            Err(DomainError::NotTokenOwner)
        );
    }
}

#[test]
fn a_revoked_token_cannot_be_changed() {
    let anna = user(Role::Member);
    let mut token = token_of(&anna);
    token.revoked = true;
    assert_eq!(
        change_api_token(
            &token,
            &anna,
            change("CLI", DAY, token.grants.clone()),
            now()
        ),
        Err(DomainError::TokenRevoked)
    );
}

#[test]
fn an_expired_token_gets_a_new_last_day() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let later = now() + 10 * DAY; // the token expired at now() + DAY
    let events = change_api_token(
        &token,
        &anna,
        TokenChange {
            name: token.name.clone(),
            expires_at: later + 30 * DAY,
            grants: token.grants.clone(),
        },
        later,
    )
    .unwrap();
    assert!(matches!(
        &events[..],
        [ApiTokenEvent::ApiTokenChanged { expires_at, .. }] if *expires_at == later + 30 * DAY
    ));
}

#[test]
fn an_unchanged_token_yields_no_events() {
    let anna = user(Role::Member);
    let token = token_of(&anna);
    let same = TokenChange {
        name: format!(" {} ", token.name),
        expires_at: token.expires_at,
        grants: token.grants.clone(),
    };
    assert_eq!(
        change_api_token(&token, &anna, same, now()).unwrap(),
        vec![]
    );
}

#[test]
fn a_token_request_is_stored_with_its_action() {
    let request = TokenRequest::Change {
        token_id: Uuid::nil(),
        change: change("CLI", DAY, vec![grant(Uuid::nil(), &[Scope::VatRead])]),
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["action"], "change");
    assert_eq!(
        serde_json::from_value::<TokenRequest>(json).unwrap(),
        request
    );
    assert_eq!(request.change().name, "CLI");
}

#[test]
fn the_owner_or_an_admin_revokes_a_token() {
    let anna = user(Role::Member);
    let admin = user(Role::Admin);
    let token = token_of(&anna);

    assert_eq!(
        revoke_api_token(&token, &anna).unwrap(),
        vec![ApiTokenEvent::ApiTokenRevoked {
            revoked_by: anna.id
        }]
    );
    assert_eq!(
        revoke_api_token(&token, &admin).unwrap(),
        vec![ApiTokenEvent::ApiTokenRevoked {
            revoked_by: admin.id
        }]
    );
}

#[test]
fn someone_else_cannot_revoke_a_token() {
    let token = token_of(&user(Role::Member));
    assert_eq!(
        revoke_api_token(&token, &user(Role::Member)),
        Err(DomainError::NotTokenOwner)
    );
}

#[test]
fn a_revoked_token_is_revoked_once() {
    let anna = user(Role::Member);
    let created = create_api_token(
        &anna,
        cmd(
            "Agent",
            DAY,
            vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])],
        ),
        "h".into(),
        now(),
    )
    .unwrap();
    let revoked = ApiTokenEvent::ApiTokenRevoked {
        revoked_by: anna.id,
    };
    let token = ApiToken::from_events(&[created, revoked]).unwrap();

    assert!(token.revoked);
    assert_eq!(revoke_api_token(&token, &anna).unwrap(), vec![]);
}
