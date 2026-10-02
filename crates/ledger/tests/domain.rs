use doris_ledger::domain::*;

fn n(number: u32) -> AccountNumber {
    AccountNumber::parse(number).unwrap()
}

fn name(raw: &str) -> AccountName {
    AccountName::parse(raw).unwrap()
}

fn seeded() -> Chart {
    Chart::from_events(&[seed_chart()])
}

#[test]
fn account_numbers_are_four_digits_from_1000_to_8999() {
    assert_eq!(n(1000).get(), 1000);
    assert_eq!(n(8999).get(), 8999);
    for bad in [0, 999, 9000, 19300] {
        assert_eq!(
            AccountNumber::parse(bad),
            Err(DomainError::InvalidAccountNumber),
            "{bad}"
        );
    }
}

#[test]
fn account_names_are_trimmed_and_1_to_100_characters() {
    assert_eq!(name("  Kassa ").as_str(), "Kassa");
    assert_eq!(name(&"å".repeat(100)).as_str().chars().count(), 100);
    for bad in ["", "   ", &"å".repeat(101)] {
        assert_eq!(
            AccountName::parse(bad),
            Err(DomainError::InvalidAccountName)
        );
    }
}

#[test]
fn the_seed_is_a_bas_selection_with_every_account_active() {
    let chart = seeded();
    let count = chart.accounts().count();
    assert!((150..=250).contains(&count), "{count} accounts");
    let bank = chart.get(n(1930)).unwrap();
    assert_eq!(bank.name.as_str(), "Företagskonto/checkkonto/affärskonto");
    assert!(chart.accounts().all(|a| a.active));
    for number in [
        1510, 2440, 2611, 2641, 2650, 3001, 4010, 5010, 6570, 7510, 8999,
    ] {
        assert!(chart.get(n(number)).is_some(), "{number} missing");
    }
}

#[test]
fn an_account_is_added_once() {
    let chart = seeded();

    let events = add_account(&chart, n(1931), name("Sparkonto")).unwrap();

    assert_eq!(
        events,
        vec![ChartEvent::AccountAdded {
            number: n(1931),
            name: name("Sparkonto")
        }]
    );
    assert_eq!(
        add_account(&chart, n(1930), name("Bank")),
        Err(DomainError::AccountExists)
    );
}

#[test]
fn renaming_needs_an_existing_account_and_a_new_name() {
    let chart = seeded();

    assert_eq!(
        rename_account(&chart, n(1930), name("Bank")).unwrap(),
        vec![ChartEvent::AccountRenamed {
            number: n(1930),
            name: name("Bank")
        }]
    );
    assert_eq!(
        rename_account(
            &chart,
            n(1930),
            name("Företagskonto/checkkonto/affärskonto")
        )
        .unwrap(),
        vec![]
    );
    assert_eq!(
        rename_account(&chart, n(1999), name("Bank")),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn deactivating_and_reactivating_are_idempotent() {
    let mut chart = seeded();

    let off = set_account_active(&chart, n(1930), false).unwrap();
    assert_eq!(
        off,
        vec![ChartEvent::AccountDeactivated { number: n(1930) }]
    );
    chart.apply(&off[0]);
    assert!(!chart.get(n(1930)).unwrap().active);
    assert_eq!(set_account_active(&chart, n(1930), false).unwrap(), vec![]);

    let on = set_account_active(&chart, n(1930), true).unwrap();
    assert_eq!(on, vec![ChartEvent::AccountReactivated { number: n(1930) }]);
    chart.apply(&on[0]);
    assert!(chart.get(n(1930)).unwrap().active);
    assert_eq!(
        set_account_active(&chart, n(1999), true),
        Err(DomainError::AccountNotFound)
    );
}

#[test]
fn chart_events_are_tagged_json_with_plain_numbers() {
    let json = serde_json::to_value(ChartEvent::AccountAdded {
        number: n(1931),
        name: name("Sparkonto"),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({"type": "AccountAdded", "number": 1931, "name": "Sparkonto"})
    );
}
