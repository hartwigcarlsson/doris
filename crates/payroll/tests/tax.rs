use doris_payroll::domain::DomainError;
use doris_payroll::tax::*;

const KR: i64 = 100;

fn amount(table: u8, from: i64, to: i64, columns: [i64; 6]) -> TaxTableRow {
    TaxTableRow {
        table,
        kind: RowKind::Amount,
        from,
        to: Some(to),
        columns,
    }
}

fn percent(table: u8, from: i64, to: Option<i64>, columns: [i64; 6]) -> TaxTableRow {
    TaxTableRow {
        table,
        kind: RowKind::Percent,
        from,
        to,
        columns,
    }
}

/// A complete 2026-shaped year: tables 29–42 with Skatteverket's intervals
/// (1–2 000, then 100 kr to 20 000, then 200 kr to 80 000; percent rows from
/// 80 001, the last one open). Values are synthetic, except table 33, whose
/// rows at the boundaries below are Skatteverket's real 2026 figures.
fn year_2026() -> Vec<TaxTableRow> {
    let real_33: [(i64, [i64; 6]); 7] = [
        (1, [0, 0, 0, 0, 0, 0]),
        (2001, [150, 0, 150, 0, 150, 2]),
        (2101, [152, 0, 150, 2, 152, 36]),
        (19901, [3391, 3316, 1613, 3391, 5498, 5498]),
        (20001, [3439, 3364, 1631, 3439, 5568, 5568]),
        (34801, [7134, 6986, 3881, 7134, 10918, 10918]),
        (79801, [26595, 26467, 23362, 23386, 30888, 30888]),
    ];
    let mut rows = Vec::new();
    for table in 29..=42u8 {
        let mut bands = vec![(1, 2000)];
        bands.extend((2001..20000).step_by(100).map(|f| (f, f + 99)));
        bands.extend((20001..80000).step_by(200).map(|f| (f, f + 199)));
        for (from, to) in bands {
            let synthetic = [1, 2, 3, 4, 5, 6].map(|k| from / 5 + k);
            let columns = match real_33.iter().find(|(f, _)| table == 33 && *f == from) {
                Some((_, real)) => *real,
                None => synthetic,
            };
            rows.push(amount(table, from, to, columns));
        }
        let first = if table == 33 {
            [33, 33, 29, 29, 39, 39]
        } else {
            [33; 6]
        };
        rows.push(percent(table, 80001, Some(81000), first));
        rows.push(percent(table, 81001, Some(1269000), [40; 6]));
        let last = if table == 33 {
            [52, 52, 52, 44, 52, 52]
        } else {
            [52; 6]
        };
        rows.push(percent(table, 1269001, None, last));
    }
    rows
}

fn table_2026() -> TaxTable {
    TaxTable::validate(2026, year_2026()).unwrap()
}

fn t33(column: u32) -> TaxSetting {
    TaxSetting::table(33, column).unwrap()
}

fn tax(setting: TaxSetting, gross: i64) -> i64 {
    preliminary_tax(setting, 2026, Some(&table_2026()), gross)
        .unwrap()
        .0
}

#[test]
fn a_setting_is_table_29_to_42_with_column_1_to_6_or_0_to_100_percent() {
    assert!(TaxSetting::table(29, 1).is_ok());
    assert!(TaxSetting::table(42, 6).is_ok());
    for (t, c) in [(28, 1), (43, 1), (33, 0), (33, 7)] {
        assert_eq!(
            TaxSetting::table(t, c),
            Err(DomainError::InvalidTaxTable),
            "{t}/{c}"
        );
    }
    assert_eq!(
        TaxSetting::percent(0).unwrap(),
        TaxSetting::Percent { percent: 0 }
    );
    assert!(TaxSetting::percent(100).is_ok());
    assert_eq!(
        TaxSetting::percent(101),
        Err(DomainError::InvalidTaxPercent)
    );
}

#[test]
fn a_complete_year_validates() {
    let table = table_2026();
    assert_eq!(table.year(), 2026);
    assert_eq!(table.rows().len(), 14 * (1 + 180 + 300 + 3));
}

#[test]
fn an_incomplete_or_odd_year_is_refused() {
    let without = |pred: &dyn Fn(&TaxTableRow) -> bool| {
        year_2026()
            .into_iter()
            .filter(|r| !pred(r))
            .collect::<Vec<_>>()
    };
    let refused = |rows: Vec<TaxTableRow>| TaxTable::validate(2026, rows).is_err();

    assert!(refused(without(&|r| r.table == 40)), "a missing table");
    assert!(
        refused(without(&|r| r.table == 33 && r.from == 34801)),
        "a gap"
    );
    assert!(
        refused(without(&|r| r.table == 33 && r.from == 79801)),
        "amounts end before 80 000"
    );
    assert!(
        refused(without(&|r| r.table == 33 && r.from == 1269001)),
        "no open percent row"
    );
    let mut overlap = year_2026();
    overlap.push(amount(33, 34900, 35100, [0; 6]));
    assert!(refused(overlap), "an overlap");
    let mut two_open = year_2026();
    two_open.push(percent(33, 1269001, None, [52; 6]));
    assert!(refused(two_open), "two open percent rows");
    let mut unknown = year_2026();
    unknown.push(amount(43, 1, 80000, [0; 6]));
    assert!(refused(unknown), "an unknown table");
    assert!(refused(vec![]), "nothing");
}

#[test]
fn a_salary_up_to_80000_kr_takes_the_amount_from_its_row() {
    assert_eq!(tax(t33(1), 35_000 * KR), 7_134 * KR);
    assert_eq!(tax(t33(3), 35_000 * KR), 3_881 * KR);
    assert_eq!(tax(t33(1), 2_000 * KR), 0);
    assert_eq!(tax(t33(1), 2_001 * KR), 150 * KR);
    assert_eq!(tax(t33(1), 20_000 * KR), 3_391 * KR);
    assert_eq!(tax(t33(1), 20_001 * KR), 3_439 * KR);
    assert_eq!(tax(t33(1), 80_000 * KR), 26_595 * KR);
    // Öre are dropped: 35 000,99 kr is 35 000 kr, and 80 000,50 kr is
    // still an amount row, not a percentage.
    assert_eq!(tax(t33(1), 35_000 * KR + 99), 7_134 * KR);
    assert_eq!(tax(t33(1), 80_000 * KR + 50), 26_595 * KR);
    assert_eq!(tax(t33(1), 99), 0);
}

#[test]
fn above_80000_kr_the_percentage_applies_to_the_whole_income() {
    // 33 % of 80 001 kr = 26 400,33 → 26 400 kr.
    assert_eq!(tax(t33(1), 80_001 * KR), 26_400 * KR);
    // Column 3 has 29 %: 23 200,29 → 23 200 kr.
    assert_eq!(tax(t33(3), 80_001 * KR), 23_200 * KR);
    // The open top row: 52 % of 2 000 000 kr.
    assert_eq!(tax(t33(1), 2_000_000 * KR), 1_040_000 * KR);
}

#[test]
fn the_basis_names_year_table_and_column() {
    let (_, basis) = preliminary_tax(t33(1), 2026, Some(&table_2026()), 35_000 * KR).unwrap();
    assert_eq!(
        basis,
        TaxBasis::Table {
            year: 2026,
            table: 33,
            column: 1
        }
    );
}

#[test]
fn a_table_setting_without_that_years_table_is_missing() {
    assert_eq!(
        preliminary_tax(t33(1), 2026, None, 35_000 * KR),
        Err(DomainError::TaxTableMissing(2026))
    );
    // A stored table for another year doesn't count.
    assert_eq!(
        preliminary_tax(t33(1), 2027, Some(&table_2026()), 35_000 * KR),
        Err(DomainError::TaxTableMissing(2027))
    );
}

#[test]
fn a_fixed_percentage_needs_no_table_and_rounds_down() {
    let thirty = TaxSetting::percent(30).unwrap();
    // 30 % of 12 345 kr (12 345,67 with öre dropped) = 3 703,5 → 3 703 kr.
    assert_eq!(
        preliminary_tax(thirty, 2026, None, 1_234_567),
        Ok((3_703 * KR, TaxBasis::Percent { percent: 30 }))
    );
    assert_eq!(
        preliminary_tax(TaxSetting::percent(0).unwrap(), 2026, None, 1_234_567)
            .unwrap()
            .0,
        0
    );
    // 100 % takes the whole kronor, never more than the gross.
    assert_eq!(
        preliminary_tax(TaxSetting::percent(100).unwrap(), 2026, None, 1_234_567)
            .unwrap()
            .0,
        1_234_500
    );
}

#[test]
fn settings_and_bases_serialize_with_a_kind_tag() {
    assert_eq!(
        serde_json::to_string(&TaxBasis::Table {
            year: 2026,
            table: 33,
            column: 1
        })
        .unwrap(),
        r#"{"kind":"table","year":2026,"table":33,"column":1}"#
    );
    assert_eq!(
        serde_json::to_string(&TaxBasis::Manual).unwrap(),
        r#"{"kind":"manual"}"#
    );
    assert_eq!(
        serde_json::from_str::<TaxSetting>(r#"{"kind":"percent","percent":30}"#).unwrap(),
        TaxSetting::Percent { percent: 30 }
    );
    assert_eq!(TaxBasis::default(), TaxBasis::Manual);
}

#[test]
fn table_tax_never_exceeds_gross() {
    let mut rows = year_2026();
    for r in rows
        .iter_mut()
        .filter(|r| r.table == 33 && r.kind == RowKind::Amount && r.from == 2001)
    {
        r.columns = [9999; 6];
    }
    let table = TaxTable::validate(2026, rows).unwrap();
    // 2 050 kr income: the row says 9 999 kr, so the whole gross is taken.
    let (tax, _) = preliminary_tax(t33(1), 2026, Some(&table), 2050 * KR).unwrap();
    assert_eq!(tax, 2050 * KR);
}

#[test]
fn a_negative_amount_or_an_impossible_percentage_is_refused() {
    let mut negative = year_2026();
    negative[5].columns[2] = -1;
    assert!(TaxTable::validate(2026, negative).is_err());
    let mut over = year_2026();
    over.iter_mut()
        .find(|r| r.kind == RowKind::Percent)
        .unwrap()
        .columns[0] = 101;
    assert!(TaxTable::validate(2026, over).is_err());
}
