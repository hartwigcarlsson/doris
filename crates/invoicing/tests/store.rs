use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::domain::{CustomerForm, DomainError, SupplierForm};
use doris_invoicing::{
    Error, add_customer, add_supplier, list_customers, list_suppliers, rebuild_projections,
    set_customer_active, set_supplier_active, update_customer, update_supplier,
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

async fn company(pool: &SqlitePool, owner: Uuid, org_nr: &str) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr,
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: "2025-01-01".parse().unwrap(),
            fiscal_year_end: "2025-12-31".parse().unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap()
}

fn customer(name: &str) -> CustomerForm<'_> {
    CustomerForm {
        name,
        org_nr: "556016-0680",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "Stockholm",
        email: "ekonomi@kund.se",
        payment_terms: 30,
    }
}

fn supplier(name: &str) -> SupplierForm<'_> {
    SupplierForm {
        name,
        org_nr: "",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: "5050-1055",
        plusgiro: "",
        iban: "",
        bic: "",
    }
}

async fn event_types(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT event_type FROM events
         WHERE stream_id LIKE 'customers-%' OR stream_id LIKE 'suppliers-%'
         ORDER BY global_position",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn rows(pool: &SqlitePool, sql: &'static str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn customers_are_added_updated_and_deactivated() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    assert_eq!(
        add_customer(&pool, id, anna, &customer("Kund AB"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        add_customer(&pool, id, anna, &customer("Annan AB"))
            .await
            .unwrap(),
        2
    );
    update_customer(
        &pool,
        id,
        anna,
        1,
        &CustomerForm {
            payment_terms: 10,
            ..customer("Kund i Stockholm AB")
        },
    )
    .await
    .unwrap();
    set_customer_active(&pool, id, anna, 2, false)
        .await
        .unwrap();

    let customers = list_customers(&pool, id, anna).await.unwrap();
    assert_eq!(customers.len(), 2);
    assert_eq!(customers[0].details.name.as_str(), "Kund i Stockholm AB");
    assert_eq!(customers[0].details.payment_terms.get(), 10);
    assert_eq!(
        customers[0].details.email.as_ref().unwrap().as_str(),
        "ekonomi@kund.se"
    );
    assert!(customers[0].active);
    assert!(!customers[1].active);
    assert_eq!(
        event_types(&pool).await,
        [
            "CustomerAdded",
            "CustomerAdded",
            "CustomerUpdated",
            "CustomerDeactivated"
        ]
    );
}

#[tokio::test]
async fn suppliers_are_added_updated_and_deactivated() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    assert_eq!(
        add_supplier(&pool, id, anna, &supplier("Lev AB"))
            .await
            .unwrap(),
        1
    );
    update_supplier(
        &pool,
        id,
        anna,
        1,
        &SupplierForm {
            iban: "SE45 5000 0000 0583 9825 7466",
            bic: "ESSESESS",
            ..supplier("Lev AB")
        },
    )
    .await
    .unwrap();
    set_supplier_active(&pool, id, anna, 1, false)
        .await
        .unwrap();
    set_supplier_active(&pool, id, anna, 1, true).await.unwrap();

    let suppliers = list_suppliers(&pool, id, anna).await.unwrap();
    assert_eq!(
        suppliers[0].details.bankgiro.as_ref().unwrap().formatted(),
        "5050-1055"
    );
    assert_eq!(
        suppliers[0].details.iban.as_ref().unwrap().as_str(),
        "SE4550000000058398257466"
    );
    assert!(suppliers[0].active);
    assert_eq!(
        event_types(&pool).await,
        [
            "SupplierAdded",
            "SupplierUpdated",
            "SupplierDeactivated",
            "SupplierReactivated"
        ]
    );
}

#[tokio::test]
async fn invalid_details_and_unknown_numbers_write_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    let err = add_customer(&pool, id, anna, &customer(""))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvalidName)));
    let err = update_supplier(&pool, id, anna, 1, &supplier("L"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierNotFound)));
    assert!(event_types(&pool).await.is_empty());
}

#[tokio::test]
async fn a_non_member_gets_not_found() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna, "556016-0680").await;

    assert!(matches!(
        list_customers(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        add_customer(&pool, id, bo, &customer("K")).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_suppliers(&pool, Uuid::new_v4(), anna).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn numbers_are_per_company() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let a = company(&pool, anna, "556016-0680").await;
    let b = company(&pool, anna, "556036-0793").await;

    add_customer(&pool, a, anna, &customer("A:s kund"))
        .await
        .unwrap();
    let err = update_customer(&pool, b, anna, 1, &customer("Fel"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerNotFound)));
    assert_eq!(
        add_customer(&pool, b, anna, &customer("B:s kund"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        add_supplier(&pool, a, anna, &supplier("Lev"))
            .await
            .unwrap(),
        1
    );

    assert_eq!(
        list_customers(&pool, a, anna).await.unwrap()[0]
            .details
            .name
            .as_str(),
        "A:s kund"
    );
}

#[tokio::test]
async fn the_projections_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;
    add_customer(&pool, id, anna, &customer("K")).await.unwrap();
    update_customer(&pool, id, anna, 1, &customer("K2"))
        .await
        .unwrap();
    add_supplier(&pool, id, anna, &supplier("L")).await.unwrap();
    set_supplier_active(&pool, id, anna, 1, false)
        .await
        .unwrap();
    let customers = "SELECT company_id || number || details || active FROM customers ORDER BY company_id, number";
    let suppliers = "SELECT company_id || number || details || active FROM suppliers ORDER BY company_id, number";
    let before = (rows(&pool, customers).await, rows(&pool, suppliers).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(
        (rows(&pool, customers).await, rows(&pool, suppliers).await),
        before
    );
    assert_eq!((before.0.len(), before.1.len()), (1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_adds_get_numbers_1_to_n() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("parties.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "556016-0680").await;

    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(
                async move { add_customer(&pool, id, anna, &customer("K")).await.unwrap() },
            )
        })
        .collect();
    let mut numbers = Vec::new();
    for task in tasks {
        numbers.push(task.await.unwrap());
    }
    numbers.sort();

    assert_eq!(numbers, (1..=20).collect::<Vec<u32>>());
    let listed: Vec<u32> = list_customers(&pool, id, anna)
        .await
        .unwrap()
        .iter()
        .map(|c| c.number)
        .collect();
    assert_eq!(listed, numbers);
}
