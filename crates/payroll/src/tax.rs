//! Preliminary tax (A-skatt) from Skatteverket's monthly tables or a fixed
//! percentage. Pure: the tables are handed in, never fetched here.

use crate::domain::DomainError;
use serde::{Deserialize, Serialize};

const TABLES: std::ops::RangeInclusive<u8> = 29..=42;
/// The monthly table gives kronor up to here, a percentage above.
const AMOUNT_LIMIT: i64 = 80_000;

/// How an employee's tax is computed (from their A-skattsedel or a
/// jämkningsbeslut).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaxSetting {
    Table { table: u8, column: u8 },
    Percent { percent: u8 },
}

impl TaxSetting {
    pub fn table(table: u32, column: u32) -> Result<Self, DomainError> {
        match (u8::try_from(table), u8::try_from(column)) {
            (Ok(table), Ok(column)) if TABLES.contains(&table) && (1..=6).contains(&column) => {
                Ok(Self::Table { table, column })
            }
            _ => Err(DomainError::InvalidTaxTable),
        }
    }

    pub fn percent(percent: u32) -> Result<Self, DomainError> {
        match u8::try_from(percent) {
            Ok(percent) if percent <= 100 => Ok(Self::Percent { percent }),
            _ => Err(DomainError::InvalidTaxPercent),
        }
    }
}

/// How a locked line's tax came about. Lines from before this existed are
/// `Manual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaxBasis {
    Table {
        year: i16,
        table: u8,
        column: u8,
    },
    Percent {
        percent: u8,
    },
    #[default]
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// `columns` are kronor ("30B").
    Amount,
    /// `columns` are percent of the whole income ("30%").
    Percent,
}

/// One income band of one table; incomes in whole kronor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxTableRow {
    pub table: u8,
    pub kind: RowKind,
    pub from: i64,
    /// `None`: no upper limit (the last percent row).
    pub to: Option<i64>,
    pub columns: [i64; 6],
}

/// A whole year's monthly tables, checked to be complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxTable {
    year: i16,
    rows: Vec<TaxTableRow>,
}

/// Why a fetched year was refused. For the log; tables hold no personal data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TaxTableError(pub String);

impl TaxTable {
    pub fn validate(year: i16, rows: Vec<TaxTableRow>) -> Result<Self, TaxTableError> {
        let fail = |why: String| Err(TaxTableError(format!("{year}: {why}")));
        if let Some(r) = rows.iter().find(|r| !TABLES.contains(&r.table)) {
            return fail(format!("unknown table {}", r.table));
        }
        let bad_column = |r: &TaxTableRow| {
            let limit = match r.kind {
                RowKind::Amount => i64::MAX,
                RowKind::Percent => 100,
            };
            r.columns.iter().any(|c| !(0..=limit).contains(c))
        };
        if rows.iter().any(bad_column) {
            return fail("a column value out of range".to_owned());
        }
        for table in TABLES {
            let bands = |kind: RowKind| {
                let mut b: Vec<_> = rows
                    .iter()
                    .filter(|r| r.table == table && r.kind == kind)
                    .collect();
                b.sort_by_key(|r| r.from);
                b
            };
            let amounts = bands(RowKind::Amount);
            let percents = bands(RowKind::Percent);
            if !contiguous(&amounts, 1) || amounts.last().and_then(|r| r.to) != Some(AMOUNT_LIMIT) {
                return fail(format!(
                    "table {table}: amounts don't cover 1-{AMOUNT_LIMIT}"
                ));
            }
            let open = percents.iter().filter(|r| r.to.is_none()).count();
            if !contiguous(&percents, AMOUNT_LIMIT + 1)
                || open != 1
                || percents.last().is_some_and(|r| r.to.is_some())
            {
                return fail(format!(
                    "table {table}: percentages don't run from {} up",
                    AMOUNT_LIMIT + 1
                ));
            }
        }
        Ok(Self { year, rows })
    }

    pub fn year(&self) -> i16 {
        self.year
    }

    pub fn rows(&self) -> &[TaxTableRow] {
        &self.rows
    }

    fn row(&self, table: u8, kind: RowKind, income: i64) -> Option<&TaxTableRow> {
        self.rows.iter().find(|r| {
            r.table == table
                && r.kind == kind
                && r.from <= income
                && r.to.is_none_or(|to| income <= to)
        })
    }
}

/// Bands sorted by `from`, starting at `start`, each beginning right after
/// the previous one ends; only the last may be open.
fn contiguous(bands: &[&TaxTableRow], start: i64) -> bool {
    let mut next = start;
    for (i, band) in bands.iter().enumerate() {
        if band.from != next {
            return false;
        }
        match band.to {
            Some(to) if to >= band.from => next = to + 1,
            None if i == bands.len() - 1 => {}
            _ => return false,
        }
    }
    !bands.is_empty()
}

/// Preliminary tax in öre on `gross` (öre) paid in `year`, and how it was
/// found. Whole kronor: öre in the income are dropped, percentages round
/// down.
pub fn preliminary_tax(
    setting: TaxSetting,
    year: i16,
    table: Option<&TaxTable>,
    gross: i64,
) -> Result<(i64, TaxBasis), DomainError> {
    let income = gross / 100;
    match setting {
        TaxSetting::Percent { percent } => {
            let tax = (income * i64::from(percent) / 100 * 100).min(gross);
            Ok((tax, TaxBasis::Percent { percent }))
        }
        TaxSetting::Table {
            table: number,
            column,
        } => {
            let table = table
                .filter(|t| t.year == year)
                .ok_or(DomainError::TaxTableMissing(year))?;
            let col = usize::from(column - 1);
            let kronor = if income == 0 {
                0
            } else if income <= AMOUNT_LIMIT {
                table
                    .row(number, RowKind::Amount, income)
                    .expect("validated")
                    .columns[col]
            } else {
                let percent = table
                    .row(number, RowKind::Percent, income)
                    .expect("validated")
                    .columns[col];
                income * percent / 100
            };
            Ok((
                (kronor * 100).min(gross),
                TaxBasis::Table {
                    year,
                    table: number,
                    column,
                },
            ))
        }
    }
}
