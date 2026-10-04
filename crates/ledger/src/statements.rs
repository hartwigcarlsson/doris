//! Resultat- och balansräkning: the saldobalans set out under the headings
//! of ÅRL's abbreviated forms (bilaga 1 and 2) as K2 uses them. This is the
//! only place that maps a BAS account to a post; the ranges follow BAS's
//! SRU codes for INK2R.

use crate::domain::{TrialBalanceRow, result_account};
use crate::{Error, Result};
use doris_company::domain::LegalForm;
use jiff::civil::Date;
use std::collections::BTreeMap;

/// One post of the resultaträkning or the balansräkning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Post {
    // Resultaträkning
    NetSales,
    InventoryChange,
    CapitalizedWork,
    OtherOperatingIncome,
    RawMaterials,
    Goods,
    OtherExternalExpenses,
    Personnel,
    Depreciation,
    CurrentAssetWritedowns,
    OtherOperatingExpenses,
    GroupShares,
    AssociateShares,
    OtherSecurities,
    InterestIncome,
    InterestExpenses,
    Appropriations,
    IncomeTax,
    OtherTaxes,
    // Balansräkning
    Intangible,
    Buildings,
    LeaseholdImprovements,
    Machinery,
    ConstructionInProgress,
    FinancialFixed,
    Inventory,
    Receivables,
    OtherReceivables,
    Prepaid,
    ShortTermInvestments,
    Cash,
    RestrictedEquity,
    RetainedEarnings,
    OwnersEquity,
    ResultForYear,
    UntaxedReserves,
    Provisions,
    LongTermLiabilities,
    Payables,
    TaxLiabilities,
    OtherCurrentLiabilities,
    Accrued,
}

/// The posts a subtotal adds up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    OperatingIncome,
    OperatingExpenses,
    Financial,
    Appropriations,
    Taxes,
    FixedAssets,
    CurrentAssets,
    Equity,
    UntaxedReserves,
    Provisions,
    LongTerm,
    ShortTerm,
}

impl Post {
    pub fn label(self) -> &'static str {
        match self {
            Post::NetSales => "Nettoomsättning",
            Post::InventoryChange => {
                "Förändring av lager av produkter i arbete, färdiga varor och pågående arbete för annans räkning"
            }
            Post::CapitalizedWork => "Aktiverat arbete för egen räkning",
            Post::OtherOperatingIncome => "Övriga rörelseintäkter",
            Post::RawMaterials => "Råvaror och förnödenheter",
            Post::Goods => "Handelsvaror",
            Post::OtherExternalExpenses => "Övriga externa kostnader",
            Post::Personnel => "Personalkostnader",
            Post::Depreciation => {
                "Av- och nedskrivningar av materiella och immateriella anläggningstillgångar"
            }
            Post::CurrentAssetWritedowns => {
                "Nedskrivningar av omsättningstillgångar utöver normala nedskrivningar"
            }
            Post::OtherOperatingExpenses => "Övriga rörelsekostnader",
            Post::GroupShares => "Resultat från andelar i koncernföretag",
            Post::AssociateShares => {
                "Resultat från andelar i intresseföretag och gemensamt styrda företag"
            }
            Post::OtherSecurities => {
                "Resultat från övriga värdepapper och fordringar som är anläggningstillgångar"
            }
            Post::InterestIncome => "Övriga ränteintäkter och liknande resultatposter",
            Post::InterestExpenses => "Räntekostnader och liknande resultatposter",
            Post::Appropriations => "Bokslutsdispositioner",
            Post::IncomeTax => "Skatt på årets resultat",
            Post::OtherTaxes => "Övriga skatter",
            Post::Intangible => "Immateriella anläggningstillgångar",
            Post::Buildings => "Byggnader och mark",
            Post::LeaseholdImprovements => "Förbättringsutgifter på annans fastighet",
            Post::Machinery => "Maskiner, inventarier och övriga materiella anläggningstillgångar",
            Post::ConstructionInProgress => {
                "Pågående nyanläggningar och förskott avseende materiella anläggningstillgångar"
            }
            Post::FinancialFixed => "Finansiella anläggningstillgångar",
            Post::Inventory => "Varulager m.m.",
            Post::Receivables => "Kundfordringar",
            Post::OtherReceivables => "Övriga fordringar",
            Post::Prepaid => "Förutbetalda kostnader och upplupna intäkter",
            Post::ShortTermInvestments => "Kortfristiga placeringar",
            Post::Cash => "Kassa och bank",
            Post::RestrictedEquity => "Bundet eget kapital",
            Post::RetainedEarnings => "Balanserat resultat",
            Post::OwnersEquity => "Eget kapital",
            Post::ResultForYear => "Årets resultat",
            Post::UntaxedReserves => "Obeskattade reserver",
            Post::Provisions => "Avsättningar",
            Post::LongTermLiabilities => "Långfristiga skulder",
            Post::Payables => "Leverantörsskulder",
            Post::TaxLiabilities => "Skatteskulder",
            Post::OtherCurrentLiabilities => "Övriga kortfristiga skulder",
            Post::Accrued => "Upplupna kostnader och förutbetalda intäkter",
        }
    }

    fn group(self) -> Group {
        match self {
            Post::NetSales
            | Post::InventoryChange
            | Post::CapitalizedWork
            | Post::OtherOperatingIncome => Group::OperatingIncome,
            Post::RawMaterials
            | Post::Goods
            | Post::OtherExternalExpenses
            | Post::Personnel
            | Post::Depreciation
            | Post::CurrentAssetWritedowns
            | Post::OtherOperatingExpenses => Group::OperatingExpenses,
            Post::GroupShares
            | Post::AssociateShares
            | Post::OtherSecurities
            | Post::InterestIncome
            | Post::InterestExpenses => Group::Financial,
            Post::Appropriations => Group::Appropriations,
            Post::IncomeTax | Post::OtherTaxes => Group::Taxes,
            Post::Intangible
            | Post::Buildings
            | Post::LeaseholdImprovements
            | Post::Machinery
            | Post::ConstructionInProgress
            | Post::FinancialFixed => Group::FixedAssets,
            Post::Inventory
            | Post::Receivables
            | Post::OtherReceivables
            | Post::Prepaid
            | Post::ShortTermInvestments
            | Post::Cash => Group::CurrentAssets,
            Post::RestrictedEquity
            | Post::RetainedEarnings
            | Post::OwnersEquity
            | Post::ResultForYear => Group::Equity,
            Post::UntaxedReserves => Group::UntaxedReserves,
            Post::Provisions => Group::Provisions,
            Post::LongTermLiabilities => Group::LongTerm,
            Post::Payables
            | Post::TaxLiabilities
            | Post::OtherCurrentLiabilities
            | Post::Accrued => Group::ShortTerm,
        }
    }

    /// Assets show a debit balance as positive; everything else in the
    /// balansräkning shows a credit balance as positive.
    fn is_asset(self) -> bool {
        matches!(self.group(), Group::FixedAssets | Group::CurrentAssets)
    }
}

/// The resultaträkning post of `account`. None for 8990–8999, where the
/// result voucher only moves the result to equity, and for 1000–2999.
// ponytail: 4000–4799 count as råvaror; a trading company may want them
// under handelsvaror, which needs a per-company choice.
pub fn income_post(account: u32) -> Option<Post> {
    Some(match account {
        3000..=3799 => Post::NetSales,
        3800..=3899 => Post::CapitalizedWork,
        3900..=3999 => Post::OtherOperatingIncome,
        4940..=4959 | 4970..=4979 => Post::InventoryChange,
        4960..=4969 => Post::Goods,
        4000..=4999 => Post::RawMaterials,
        5000..=6999 => Post::OtherExternalExpenses,
        7000..=7699 => Post::Personnel,
        7740..=7749 | 7790..=7799 => Post::CurrentAssetWritedowns,
        7700..=7899 => Post::Depreciation,
        7900..=7999 => Post::OtherOperatingExpenses,
        8000..=8099 => Post::GroupShares,
        8100..=8199 => Post::AssociateShares,
        8200..=8299 => Post::OtherSecurities,
        8300..=8399 => Post::InterestIncome,
        8400..=8799 => Post::InterestExpenses,
        8800..=8899 => Post::Appropriations,
        8900..=8979 => Post::IncomeTax,
        8980..=8989 => Post::OtherTaxes,
        _ => return None,
    })
}

/// The balansräkning post of `account`. The resultaträkning's accounts
/// (3000–8989) fold into "Årets resultat". The result account (2099, or
/// 2019) and 8990–8999 go with the earlier results: closing books 8999
/// against the result account, so the two cancel there, and a closed year
/// reads like an open one.
pub fn balance_post(account: u32, legal_form: LegalForm) -> Option<Post> {
    let result = u32::from(result_account(legal_form).get());
    Some(match account {
        1000..=1099 => Post::Intangible,
        1120..=1129 => Post::LeaseholdImprovements,
        1180..=1189 | 1280..=1289 => Post::ConstructionInProgress,
        1100..=1199 => Post::Buildings,
        1200..=1299 => Post::Machinery,
        1300..=1399 => Post::FinancialFixed,
        1400..=1499 => Post::Inventory,
        1500..=1599 => Post::Receivables,
        1600..=1699 => Post::OtherReceivables,
        1700..=1799 => Post::Prepaid,
        1800..=1899 => Post::ShortTermInvestments,
        1900..=1999 => Post::Cash,
        3000..=8989 => Post::ResultForYear,
        // Enskild firma, HB and KB: the owners' capital is one post. The
        // result account and the result voucher's 8999 sit with the earlier
        // results, so "Årets resultat" always equals the resultaträkning's.
        2000..=2099 | 8990..=8999 if result == 2019 => Post::OwnersEquity,
        8990..=8999 => Post::RetainedEarnings,
        2080..=2089 => Post::RestrictedEquity,
        2000..=2099 => Post::RetainedEarnings,
        2100..=2199 => Post::UntaxedReserves,
        2200..=2299 => Post::Provisions,
        2300..=2399 => Post::LongTermLiabilities,
        2440..=2449 => Post::Payables,
        2500..=2599 => Post::TaxLiabilities,
        2900..=2999 => Post::Accrued,
        2400..=2899 => Post::OtherCurrentLiabilities,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    Item,
    Subtotal,
}

/// One line of a statement, in öre with the sign the statement shows.
/// Headings carry no amounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementLine {
    pub label: &'static str,
    pub kind: LineKind,
    pub amount: i64,
    /// None when there is no previous year.
    pub previous: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialStatements {
    pub income: Vec<StatementLine>,
    pub balance: Vec<StatementLine>,
    /// Summa tillgångar − summa eget kapital och skulder: 0 unless an
    /// earlier year is open, so its result never reached equity.
    pub difference: i64,
    pub previous_difference: Option<i64>,
    pub previous_fiscal_year_start: Option<Date>,
}

#[derive(Clone, Copy)]
enum Entry {
    Heading(&'static str),
    Item(Post),
    Subtotal(&'static str, &'static [Group]),
}

const ASSET_GROUPS: &[Group] = &[Group::FixedAssets, Group::CurrentAssets];
const CLAIM_GROUPS: &[Group] = &[
    Group::Equity,
    Group::UntaxedReserves,
    Group::Provisions,
    Group::LongTerm,
    Group::ShortTerm,
];

/// Kostnadsslagsindelad resultaträkning, ÅRL bilaga 2.
const INCOME: &[Entry] = &[
    Entry::Heading("Rörelseintäkter, lagerförändringar m.m."),
    Entry::Item(Post::NetSales),
    Entry::Item(Post::InventoryChange),
    Entry::Item(Post::CapitalizedWork),
    Entry::Item(Post::OtherOperatingIncome),
    Entry::Subtotal(
        "Summa rörelseintäkter, lagerförändringar m.m.",
        &[Group::OperatingIncome],
    ),
    Entry::Heading("Rörelsekostnader"),
    Entry::Item(Post::RawMaterials),
    Entry::Item(Post::Goods),
    Entry::Item(Post::OtherExternalExpenses),
    Entry::Item(Post::Personnel),
    Entry::Item(Post::Depreciation),
    Entry::Item(Post::CurrentAssetWritedowns),
    Entry::Item(Post::OtherOperatingExpenses),
    Entry::Subtotal("Summa rörelsekostnader", &[Group::OperatingExpenses]),
    Entry::Subtotal(
        "Rörelseresultat",
        &[Group::OperatingIncome, Group::OperatingExpenses],
    ),
    Entry::Heading("Finansiella poster"),
    Entry::Item(Post::GroupShares),
    Entry::Item(Post::AssociateShares),
    Entry::Item(Post::OtherSecurities),
    Entry::Item(Post::InterestIncome),
    Entry::Item(Post::InterestExpenses),
    Entry::Subtotal("Summa finansiella poster", &[Group::Financial]),
    Entry::Subtotal(
        "Resultat efter finansiella poster",
        &[
            Group::OperatingIncome,
            Group::OperatingExpenses,
            Group::Financial,
        ],
    ),
    Entry::Item(Post::Appropriations),
    Entry::Subtotal(
        "Resultat före skatt",
        &[
            Group::OperatingIncome,
            Group::OperatingExpenses,
            Group::Financial,
            Group::Appropriations,
        ],
    ),
    Entry::Item(Post::IncomeTax),
    Entry::Item(Post::OtherTaxes),
    Entry::Subtotal(
        "Årets resultat",
        &[
            Group::OperatingIncome,
            Group::OperatingExpenses,
            Group::Financial,
            Group::Appropriations,
            Group::Taxes,
        ],
    ),
];

/// Balansräkning, ÅRL bilaga 1, up to equity.
const ASSETS: &[Entry] = &[
    Entry::Heading("Tillgångar"),
    Entry::Heading("Anläggningstillgångar"),
    Entry::Item(Post::Intangible),
    Entry::Item(Post::Buildings),
    Entry::Item(Post::LeaseholdImprovements),
    Entry::Item(Post::Machinery),
    Entry::Item(Post::ConstructionInProgress),
    Entry::Item(Post::FinancialFixed),
    Entry::Subtotal("Summa anläggningstillgångar", &[Group::FixedAssets]),
    Entry::Heading("Omsättningstillgångar"),
    Entry::Item(Post::Inventory),
    Entry::Item(Post::Receivables),
    Entry::Item(Post::OtherReceivables),
    Entry::Item(Post::Prepaid),
    Entry::Item(Post::ShortTermInvestments),
    Entry::Item(Post::Cash),
    Entry::Subtotal("Summa omsättningstillgångar", &[Group::CurrentAssets]),
    Entry::Subtotal("Summa tillgångar", ASSET_GROUPS),
    Entry::Heading("Eget kapital och skulder"),
];

/// Aktiebolag, ekonomisk förening and the other forms.
const COMPANY_EQUITY: &[Entry] = &[
    Entry::Heading("Eget kapital"),
    Entry::Item(Post::RestrictedEquity),
    Entry::Heading("Fritt eget kapital"),
    Entry::Item(Post::RetainedEarnings),
    Entry::Item(Post::ResultForYear),
    Entry::Subtotal("Summa eget kapital", &[Group::Equity]),
];

/// Enskild firma, HB and KB.
const OWNERS_EQUITY: &[Entry] = &[
    Entry::Item(Post::OwnersEquity),
    Entry::Item(Post::ResultForYear),
    Entry::Subtotal("Summa eget kapital", &[Group::Equity]),
];

const LIABILITIES: &[Entry] = &[
    Entry::Item(Post::UntaxedReserves),
    Entry::Item(Post::Provisions),
    Entry::Item(Post::LongTermLiabilities),
    Entry::Heading("Kortfristiga skulder"),
    Entry::Item(Post::Payables),
    Entry::Item(Post::TaxLiabilities),
    Entry::Item(Post::OtherCurrentLiabilities),
    Entry::Item(Post::Accrued),
    Entry::Subtotal("Summa kortfristiga skulder", &[Group::ShortTerm]),
    Entry::Subtotal("Summa eget kapital och skulder", CLAIM_GROUPS),
];

type Totals = BTreeMap<Post, i64>;

/// The resultaträkning and balansräkning of `current`, with `previous`
/// (its start and saldobalans) as the comparison year.
pub fn build(
    current: &[TrialBalanceRow],
    previous: Option<(Date, &[TrialBalanceRow])>,
    legal_form: LegalForm,
) -> Result<FinancialStatements> {
    let (income_now, balance_now) = totals(current, legal_form)?;
    let before = previous
        .map(|(_, rows)| totals(rows, legal_form))
        .transpose()?;
    // The owners of an enskild firma, HB or KB keep the result on 2019.
    let equity = if result_account(legal_form).get() == 2019 {
        OWNERS_EQUITY
    } else {
        COMPANY_EQUITY
    };
    let balance_layout = [ASSETS, equity, LIABILITIES].concat();
    Ok(FinancialStatements {
        income: lines(INCOME, &income_now, before.as_ref().map(|(i, _)| i))?,
        balance: lines(
            &balance_layout,
            &balance_now,
            before.as_ref().map(|(_, b)| b),
        )?,
        difference: difference(&balance_now)?,
        previous_difference: before.as_ref().map(|(_, b)| difference(b)).transpose()?,
        previous_fiscal_year_start: previous.map(|(start, _)| start),
    })
}

/// Each post's amount, with the sign its statement shows.
fn totals(rows: &[TrialBalanceRow], legal_form: LegalForm) -> Result<(Totals, Totals)> {
    let (mut income, mut balance) = (Totals::new(), Totals::new());
    for row in rows {
        let closing = row
            .opening
            .checked_add(row.debit)
            .and_then(|b| b.checked_sub(row.credit))
            .ok_or(Error::Overflow)?;
        let credit = closing.checked_neg().ok_or(Error::Overflow)?;
        if let Some(post) = income_post(row.account) {
            add(&mut income, post, credit)?;
        }
        if let Some(post) = balance_post(row.account, legal_form) {
            add(
                &mut balance,
                post,
                if post.is_asset() { closing } else { credit },
            )?;
        }
    }
    Ok((income, balance))
}

fn add(totals: &mut Totals, post: Post, amount: i64) -> Result<()> {
    let sum = totals.entry(post).or_insert(0);
    *sum = sum.checked_add(amount).ok_or(Error::Overflow)?;
    Ok(())
}

fn sum(totals: &Totals, groups: &[Group]) -> Result<i64> {
    totals
        .iter()
        .filter(|(post, _)| groups.contains(&post.group()))
        .try_fold(0i64, |acc, (_, amount)| {
            acc.checked_add(*amount).ok_or(Error::Overflow)
        })
}

fn difference(balance: &Totals) -> Result<i64> {
    sum(balance, ASSET_GROUPS)?
        .checked_sub(sum(balance, CLAIM_GROUPS)?)
        .ok_or(Error::Overflow)
}

fn lines(layout: &[Entry], now: &Totals, before: Option<&Totals>) -> Result<Vec<StatementLine>> {
    let mut out = Vec::new();
    for entry in layout {
        let (label, kind, amount, previous) = match *entry {
            Entry::Heading(label) => (label, LineKind::Heading, 0, None),
            Entry::Item(post) => {
                let amount = now.get(&post).copied().unwrap_or(0);
                let previous = before.map(|b| b.get(&post).copied().unwrap_or(0));
                if amount == 0 && previous.unwrap_or(0) == 0 {
                    continue;
                }
                (post.label(), LineKind::Item, amount, previous)
            }
            Entry::Subtotal(label, groups) => (
                label,
                LineKind::Subtotal,
                sum(now, groups)?,
                before.map(|b| sum(b, groups)).transpose()?,
            ),
        };
        out.push(StatementLine {
            label,
            kind,
            amount,
            previous,
        });
    }
    Ok(out)
}
