use doris_company::domain::{AccountingMethod, DomainError, LegalForm};
use doris_company::{
    Error, NewCompany, add_member, get_company, get_company_in, list_companies,
    rebuild_projections, register_company,
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

fn input<'a>(org_nr: &'a str, name: &'a str) -> NewCompany<'a> {
    NewCompany {
        org_nr,
        name,
        legal_form: LegalForm::Aktiebolag,
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        fiscal_year_start: "2026-01-01".parse().unwrap(),
        fiscal_year_end: "2026-12-31".parse().unwrap(),
        accounting_method: AccountingMethod::Invoice,
    }
}

async fn event_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_registered_company_is_listed_for_its_creator_only() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());

    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB"))
        .await
        .unwrap();

    let listed = list_companies(&pool, anna).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].id,
            listed[0].org_nr.as_str(),
            listed[0].name.as_str()
        ),
        (id, "5560160680", "Exempel AB")
    );
    assert!(list_companies(&pool, bo).await.unwrap().is_empty());
    let company = get_company(&pool, id, anna).await.unwrap();
    assert_eq!(company.address.city.as_deref(), Some("Stockholm"));
    assert!(matches!(
        get_company(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_same_org_nr_cannot_be_registered_twice() {
    let pool = db().await;
    register_company(&pool, Uuid::new_v4(), input("556016-0680", "Exempel AB"))
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let again = register_company(&pool, Uuid::new_v4(), input("5560160680", "Annat AB")).await;

    assert!(matches!(again, Err(Error::AlreadyExists)));
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn invalid_input_is_rejected_before_anything_is_written() {
    let pool = db().await;
    let mut bad_year = input("556016-0680", "Exempel AB");
    bad_year.fiscal_year_end = "2026-12-30".parse().unwrap();

    for (cmd, expected) in [
        (
            input("556016-0681", "Exempel AB"),
            DomainError::InvalidOrgNr,
        ),
        (input("556016-0680", " "), DomainError::InvalidCompanyName),
        (bad_year, DomainError::InvalidFiscalYear),
    ] {
        let err = register_company(&pool, Uuid::new_v4(), cmd)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::Domain(e) if e == expected),
            "{expected:?}"
        );
    }
    assert_eq!(event_count(&pool).await, 0);
}

#[tokio::test]
async fn a_member_adds_another_user_who_then_sees_the_company() {
    let pool = db().await;
    let (anna, bo, stranger) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB"))
        .await
        .unwrap();

    add_member(&pool, id, anna, bo).await.unwrap();
    add_member(&pool, id, anna, bo).await.unwrap();

    assert_eq!(list_companies(&pool, bo).await.unwrap().len(), 1);
    assert_eq!(
        get_company(&pool, id, bo).await.unwrap().members,
        vec![anna, bo]
    );
    assert!(matches!(
        add_member(&pool, id, stranger, stranger).await,
        Err(Error::Domain(DomainError::NotMember))
    ));
    assert!(matches!(
        add_member(&pool, Uuid::new_v4(), anna, bo).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn projections_rebuild_from_the_event_log() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB"))
        .await
        .unwrap();
    register_company(&pool, bo, input("556036-0793", "Bolaget AB"))
        .await
        .unwrap();
    add_member(&pool, id, anna, bo).await.unwrap();
    #[allow(clippy::type_complexity)]
    let dump = |pool: SqlitePool| async move {
        let companies: Vec<(String, String, String, String, Option<String>, String, String, String)> =
            sqlx::query_as(
                "SELECT company_id, org_nr, name, legal_form, city, first_fiscal_year_start,
                        first_fiscal_year_end, accounting_method FROM companies ORDER BY company_id",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
        let members: Vec<(String, String)> = sqlx::query_as(
            "SELECT company_id, user_id FROM company_members ORDER BY company_id, user_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        (companies, members)
    };
    let before = dump(pool.clone()).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(dump(pool.clone()).await, before);
    assert_eq!(before.1.len(), 3);
}

#[tokio::test]
async fn companies_are_listed_alphabetically_ignoring_case() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    for (org_nr, name) in [
        ("556016-0680", "beta AB"),
        ("556036-0793", "Alfa AB"),
        ("556703-7485", "Ceta AB"),
    ] {
        register_company(&pool, anna, input(org_nr, name))
            .await
            .unwrap();
    }
    let names: Vec<_> = list_companies(&pool, anna)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["Alfa AB", "beta AB", "Ceta AB"]);
}

#[tokio::test]
async fn get_company_in_works_inside_a_write_transaction() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = register_company(&pool, anna, input("556016-0680", "Exempel AB"))
        .await
        .unwrap();

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let company = get_company_in(&mut tx, id, anna).await.unwrap();
    let outsider = get_company_in(&mut tx, id, bo).await;
    tx.rollback().await.unwrap();

    assert_eq!(company.id, id);
    assert!(matches!(outsider, Err(Error::NotFound)));
}
