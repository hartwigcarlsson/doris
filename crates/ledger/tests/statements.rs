use doris_company::domain::LegalForm;
use doris_ledger::statements::{Post, balance_post, income_post};

const FORMS: [LegalForm; 8] = [
    LegalForm::Aktiebolag,
    LegalForm::Handelsbolag,
    LegalForm::Kommanditbolag,
    LegalForm::EnskildFirma,
    LegalForm::EkonomiskForening,
    LegalForm::IdeellForening,
    LegalForm::Stiftelse,
    LegalForm::Other,
];

#[test]
fn every_account_lands_in_exactly_one_post_of_each_statement() {
    for form in FORMS {
        for account in 1000..=8999 {
            let (income, balance) = (income_post(account), balance_post(account, form));
            match account {
                1000..=2999 => assert!(income.is_none() && balance.is_some(), "{account}"),
                3000..=8989 => assert!(
                    income.is_some() && balance == Some(Post::ResultForYear),
                    "{account}"
                ),
                // The result voucher's 8999 only moves the result to equity.
                _ => assert!(
                    income.is_none() && balance == Some(Post::ResultForYear),
                    "{account}"
                ),
            }
        }
    }
    assert_eq!(income_post(999), None);
    assert_eq!(balance_post(999, LegalForm::Aktiebolag), None);
}

#[test]
fn the_narrower_ranges_win_over_their_groups() {
    let ab = LegalForm::Aktiebolag;
    assert_eq!(income_post(4010), Some(Post::RawMaterials));
    assert_eq!(income_post(4950), Some(Post::InventoryChange));
    assert_eq!(income_post(4960), Some(Post::Goods));
    assert_eq!(income_post(7832), Some(Post::Depreciation));
    assert_eq!(income_post(7745), Some(Post::CurrentAssetWritedowns));
    assert_eq!(income_post(7790), Some(Post::CurrentAssetWritedowns));
    assert_eq!(income_post(8410), Some(Post::InterestExpenses));
    assert_eq!(income_post(8910), Some(Post::IncomeTax));
    assert_eq!(income_post(8980), Some(Post::OtherTaxes));
    assert_eq!(balance_post(1125, ab), Some(Post::LeaseholdImprovements));
    assert_eq!(balance_post(1185, ab), Some(Post::ConstructionInProgress));
    assert_eq!(balance_post(1285, ab), Some(Post::ConstructionInProgress));
    assert_eq!(balance_post(1110, ab), Some(Post::Buildings));
    assert_eq!(balance_post(1220, ab), Some(Post::Machinery));
    assert_eq!(balance_post(1930, ab), Some(Post::Cash));
    assert_eq!(balance_post(2440, ab), Some(Post::Payables));
    assert_eq!(balance_post(2510, ab), Some(Post::TaxLiabilities));
    assert_eq!(balance_post(2610, ab), Some(Post::OtherCurrentLiabilities));
    assert_eq!(balance_post(2990, ab), Some(Post::Accrued));
}

#[test]
fn equity_follows_the_legal_form() {
    let (ab, ef) = (LegalForm::Aktiebolag, LegalForm::EnskildFirma);
    assert_eq!(balance_post(2081, ab), Some(Post::RestrictedEquity));
    assert_eq!(balance_post(2091, ab), Some(Post::RetainedEarnings));
    assert_eq!(balance_post(2019, ab), Some(Post::RetainedEarnings));
    assert_eq!(balance_post(2099, ab), Some(Post::ResultForYear));
    assert_eq!(balance_post(2010, ef), Some(Post::OwnersEquity));
    assert_eq!(balance_post(2099, ef), Some(Post::OwnersEquity));
    assert_eq!(balance_post(2019, ef), Some(Post::ResultForYear));
    assert_eq!(
        balance_post(2019, LegalForm::Handelsbolag),
        Some(Post::ResultForYear)
    );
    assert_eq!(
        balance_post(2081, LegalForm::Kommanditbolag),
        Some(Post::OwnersEquity)
    );
}

#[test]
fn posts_carry_their_arl_labels() {
    assert_eq!(Post::NetSales.label(), "Nettoomsättning");
    assert_eq!(Post::Cash.label(), "Kassa och bank");
    assert_eq!(Post::ResultForYear.label(), "Årets resultat");
    assert_eq!(Post::OwnersEquity.label(), "Eget kapital");
}
