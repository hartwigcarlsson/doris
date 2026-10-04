//! Skatteverket's open data: the monthly tax tables ("Skattetabeller för
//! månadslön") for one year, fetched page by page and checked before they
//! are used. No credentials, and no personal data either way.

use doris_payroll::tax::{RowKind, TaxTable, TaxTableRow};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

pub const TAX_TABLES_URL: &str =
    "https://skatteverket.entryscape.net/rowstore/dataset/88320397-5c32-4c16-ae79-d36d95b17b95";
const PAGE: usize = 500;
/// A year is about 8 000 rows; anything far beyond that is not the dataset.
const MAX_ROWS: usize = 20_000;

pub struct TaxTables {
    http: reqwest::Client,
    url: String,
}

#[derive(Deserialize)]
struct Page {
    #[serde(rename = "resultCount")]
    result_count: usize,
    results: Vec<HashMap<String, Value>>,
}

impl TaxTables {
    pub fn new(url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("the system TLS library loads");
        Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
        }
    }

    /// The whole year, or why not (for the log). An unpublished year has
    /// no rows and is refused like any other incomplete answer.
    pub async fn fetch(&self, year: i16) -> Result<TaxTable, String> {
        let mut rows = Vec::new();
        let expected = loop {
            let page = self.page(year, rows.len()).await?;
            if page.result_count > MAX_ROWS || rows.len() + page.results.len() > page.result_count {
                return Err(format!(
                    "{year}: implausible answer ({} rows)",
                    page.result_count
                ));
            }
            for row in &page.results {
                rows.push(parse_row(year, row)?);
            }
            if page.results.is_empty() || rows.len() >= page.result_count {
                break page.result_count;
            }
        };
        if rows.len() != expected {
            return Err(format!("{year}: got {} of {expected} rows", rows.len()));
        }
        TaxTable::validate(year, rows).map_err(|e| e.to_string())
    }

    async fn page(&self, year: i16, offset: usize) -> Result<Page, String> {
        let failed = |e: reqwest::Error| {
            let mut chain = String::new();
            let mut cause = std::error::Error::source(&e);
            while let Some(c) = cause {
                chain.push_str(&format!(": {c}"));
                cause = c.source();
            }
            format!("{year}: {}{chain}", e.without_url())
        };
        let url = reqwest::Url::parse_with_params(
            &self.url,
            [
                ("år", year.to_string()),
                ("_limit", PAGE.to_string()),
                ("_offset", offset.to_string()),
            ],
        )
        .map_err(|e| format!("{year}: {e}"))?;
        self.http
            .get(url)
            .send()
            .await
            .map_err(failed)?
            .error_for_status()
            .map_err(failed)?
            .json()
            .await
            .map_err(failed)
    }
}

/// One rowstore row: every value is text ("" for no upper limit).
fn parse_row(year: i16, row: &HashMap<String, Value>) -> Result<TaxTableRow, String> {
    let text = |key: &str| match row.get(key) {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let number = |key: &str| {
        text(key)
            .parse::<i64>()
            .map_err(|_| format!("{year}: bad {key:?}"))
    };
    if text("år") != year.to_string() {
        return Err(format!("{year}: a row for {:?}", text("år")));
    }
    let kind = match text("antal dgr").as_str() {
        "30B" => RowKind::Amount,
        "30%" => RowKind::Percent,
        other => return Err(format!("{year}: unknown row kind {other:?}")),
    };
    let to = match text("inkomst t.o.m.").as_str() {
        "" => None,
        _ => Some(number("inkomst t.o.m.")?),
    };
    let column = |n: usize| number(&format!("kolumn {n}"));
    Ok(TaxTableRow {
        table: u8::try_from(number("tabellnr")?).map_err(|_| format!("{year}: bad table"))?,
        kind,
        from: number("inkomst fr.o.m.")?,
        to,
        columns: [
            column(1)?,
            column(2)?,
            column(3)?,
            column(4)?,
            column(5)?,
            column(6)?,
        ],
    })
}
