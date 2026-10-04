mod common;

use common::{fake_skatteverket, tax_rows};
use doris_payroll::tax::{TaxSetting, preliminary_tax};
use doris_server::skatteverket::TaxTables;
use serde_json::json;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn a_year_is_fetched_page_by_page_and_checked() {
    let fake = fake_skatteverket(tax_rows(2026)).await;

    let table = TaxTables::new(&fake.url).fetch(2026).await.unwrap();

    assert_eq!(table.year(), 2026);
    assert_eq!(table.rows().len(), 1134);
    assert_eq!(fake.requests.load(Ordering::SeqCst), 3);
    let t33 = TaxSetting::table(33, 1).unwrap();
    assert_eq!(
        preliminary_tax(t33, 2026, Some(&table), 3_500_000)
            .unwrap()
            .0,
        713_400
    );
}

#[tokio::test]
async fn a_year_not_published_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    let err = TaxTables::new(&fake.url).fetch(2027).await.unwrap_err();
    assert!(err.contains("no rows"), "{err}");
}

#[tokio::test]
async fn an_error_answer_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.broken.store(true, Ordering::SeqCst);
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn a_year_that_stops_halfway_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.truncated.store(true, Ordering::SeqCst);
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn an_unknown_row_kind_or_a_missing_table_is_refused() {
    let mut odd = tax_rows(2026);
    odd[0]["antal dgr"] = json!("14D");
    let fake = fake_skatteverket(odd).await;
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());

    let without_40: Vec<_> = tax_rows(2026)
        .into_iter()
        .filter(|r| r["tabellnr"] != "40")
        .collect();
    let fake = fake_skatteverket(without_40).await;
    assert!(TaxTables::new(&fake.url).fetch(2026).await.is_err());
}

#[tokio::test]
async fn nothing_listening_is_refused_quickly() {
    let err = TaxTables::new("http://127.0.0.1:9/rowstore")
        .fetch(2026)
        .await
        .unwrap_err();
    assert!(err.to_lowercase().contains("refused"), "{err}");
}

#[tokio::test]
async fn a_server_that_never_ends_is_refused() {
    let fake = fake_skatteverket(tax_rows(2026)).await;
    fake.repeating.store(true, Ordering::SeqCst);
    let fetched = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        TaxTables::new(&fake.url).fetch(2026),
    )
    .await
    .expect("fetch ends");
    assert!(fetched.is_err());
}
