//! Resultat- och balansräkning: the saldobalans set out under the headings
//! of ÅRL's abbreviated forms (bilaga 1 and 2) as K2 uses them. This is the
//! only place that maps a BAS account to a post; the ranges follow BAS's
//! SRU codes for INK2R.

use crate::domain::result_account;
use doris_company::domain::LegalForm;

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
#[allow(dead_code)]
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

    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

/// The balansräkning post of `account`. Every resultaträkning account
/// (3000–8999, 8999 included) folds into "Årets resultat", so the year's
/// result shows there whether or not the year is closed.
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
        a if a == result || (3000..=8999).contains(&a) => Post::ResultForYear,
        // Enskild firma, HB and KB: the owners' capital is one post.
        2000..=2099 if result == 2019 => Post::OwnersEquity,
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
