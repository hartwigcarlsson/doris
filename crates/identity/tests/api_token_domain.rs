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
fn a_token_expires_after_now_and_within_367_days() {
    let anna = user(Role::Member);
    let grants = || vec![grant(Uuid::new_v4(), &[Scope::LedgerRead])];
    for expires_in in [
        SignedDuration::ZERO,
        -DAY,
        367 * DAY + SignedDuration::from_secs(1),
    ] {
        assert_eq!(
            create_api_token(&anna, cmd("Agent", expires_in, grants()), "h".into(), now()),
            Err(DomainError::InvalidTokenExpiry),
            "{expires_in:?}"
        );
    }
    assert!(create_api_token(&anna, cmd("Agent", 367 * DAY, grants()), "h".into(), now()).is_ok());
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
