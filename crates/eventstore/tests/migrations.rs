//! Two branches can each add the next migration number in parallel; sqlx
//! then refuses to open any database (`UNIQUE constraint failed:
//! _sqlx_migrations.version`). Catch that here, with the names.

use std::collections::BTreeMap;

#[test]
fn every_migration_has_its_own_version() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");
    let mut by_version = BTreeMap::<String, Vec<String>>::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if let Some(version) = name.strip_suffix(".sql").and_then(|n| n.split('_').next()) {
            by_version
                .entry(version.to_owned())
                .or_default()
                .push(name.clone());
        }
    }
    let shared: Vec<_> = by_version
        .values()
        .filter(|names| names.len() > 1)
        .collect();
    assert!(
        shared.is_empty(),
        "migrations sharing a version: {shared:?}"
    );
}
