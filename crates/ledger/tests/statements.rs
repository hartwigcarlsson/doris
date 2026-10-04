use doris_company::domain::LegalForm;
use doris_ledger::Error;
use doris_ledger::domain::TrialBalanceRow;
use doris_ledger::statements::{
    FinancialStatements, LineKind, Post, StatementLine, balance_post, build, income_post,
};
use jiff::civil::Date;

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

fn row(account: u32, opening: i64, debit: i64, credit: i64) -> TrialBalanceRow {
    TrialBalanceRow {
        account,
        name: String::new(),
        opening,
        debit,
        credit,
    }
}

fn line<'a>(lines: &'a [StatementLine], label: &str) -> &'a StatementLine {
    lines
        .iter()
        .find(|l| l.label == label)
        .unwrap_or_else(|| panic!("no line {label}"))
}

fn amount(lines: &[StatementLine], label: &str) -> i64 {
    line(lines, label).amount
}

fn shown(lines: &[StatementLine], label: &str) -> bool {
    lines.iter().any(|l| l.label == label)
}

/// An open year: 1 000 sold, 300 rent, paid through the bank.
fn trading() -> Vec<TrialBalanceRow> {
    vec![
        row(1930, 0, 1_000, 300),
        row(3001, 0, 0, 1_000),
        row(5010, 0, 300, 0),
    ]
}

fn ab(rows: &[TrialBalanceRow]) -> FinancialStatements {
    build(rows, None, LegalForm::Aktiebolag).unwrap()
}

#[test]
fn the_income_statement_runs_down_to_the_years_result() {
    let s = ab(&trading());
    assert_eq!(amount(&s.income, "Nettoomsättning"), 1_000);
    assert_eq!(amount(&s.income, "Övriga externa kostnader"), -300);
    assert_eq!(
        amount(&s.income, "Summa rörelseintäkter, lagerförändringar m.m."),
        1_000
    );
    assert_eq!(amount(&s.income, "Summa rörelsekostnader"), -300);
    assert_eq!(amount(&s.income, "Rörelseresultat"), 700);
    assert_eq!(amount(&s.income, "Resultat efter finansiella poster"), 700);
    assert_eq!(amount(&s.income, "Resultat före skatt"), 700);
    assert_eq!(amount(&s.income, "Årets resultat"), 700);
    assert_eq!(s.income[0].label, "Rörelseintäkter, lagerförändringar m.m.");
    assert_eq!(s.income[0].kind, LineKind::Heading);
    assert_eq!(line(&s.income, "Nettoomsättning").kind, LineKind::Item);
    assert_eq!(line(&s.income, "Rörelseresultat").kind, LineKind::Subtotal);
    assert_eq!(s.income.last().unwrap().label, "Årets resultat");
}

#[test]
fn the_balance_sheet_carries_the_result_into_equity() {
    let s = ab(&trading());
    assert_eq!(amount(&s.balance, "Kassa och bank"), 700);
    assert_eq!(amount(&s.balance, "Summa omsättningstillgångar"), 700);
    assert_eq!(amount(&s.balance, "Summa tillgångar"), 700);
    assert_eq!(amount(&s.balance, "Årets resultat"), 700);
    assert_eq!(amount(&s.balance, "Summa eget kapital"), 700);
    assert_eq!(amount(&s.balance, "Summa eget kapital och skulder"), 700);
    assert_eq!(s.difference, 0);
    assert_eq!(s.balance[0].label, "Tillgångar");
}

#[test]
fn a_closed_year_reads_exactly_like_the_open_one() {
    for (form, result_account) in [
        (LegalForm::Aktiebolag, 2099),
        (LegalForm::EnskildFirma, 2019),
        (LegalForm::Handelsbolag, 2019),
    ] {
        let open = build(&trading(), None, form).unwrap();
        // "Årets resultat": a 700 profit is debited to 8999.
        let mut rows = trading();
        rows.push(row(result_account, 0, 0, 700));
        rows.push(row(8999, 0, 700, 0));
        let closed = build(&rows, None, form).unwrap();
        assert_eq!(closed.income, open.income, "{form:?}");
        assert_eq!(closed.balance, open.balance, "{form:?}");
        assert_eq!(closed.difference, 0, "{form:?}");
        assert_eq!(amount(&closed.balance, "Årets resultat"), 700, "{form:?}");
    }
}

#[test]
fn equity_is_split_for_a_company_and_whole_for_an_owner() {
    let rows = [
        row(1930, 10_000, 1_000, 0),
        row(2010, -4_000, 0, 0),
        row(2081, -6_000, 0, 0),
        row(3001, 0, 0, 1_000),
    ];
    let company = build(&rows, None, LegalForm::Aktiebolag).unwrap();
    assert!(shown(&company.balance, "Fritt eget kapital"));
    assert_eq!(amount(&company.balance, "Bundet eget kapital"), 6_000);
    assert_eq!(amount(&company.balance, "Balanserat resultat"), 4_000);
    assert_eq!(amount(&company.balance, "Årets resultat"), 1_000);
    assert_eq!(company.difference, 0);

    let owner = build(&rows, None, LegalForm::EnskildFirma).unwrap();
    assert!(!shown(&owner.balance, "Bundet eget kapital"));
    assert!(!shown(&owner.balance, "Fritt eget kapital"));
    assert_eq!(amount(&owner.balance, "Eget kapital"), 10_000);
    assert_eq!(amount(&owner.balance, "Årets resultat"), 1_000);
    assert_eq!(amount(&owner.balance, "Summa eget kapital"), 11_000);
    assert_eq!(owner.difference, 0);
}

#[test]
fn posts_without_amounts_in_either_year_are_hidden() {
    let now = [row(1930, 0, 1_000, 0), row(3001, 0, 0, 1_000)];
    let before = trading();
    let start: Date = "2025-01-01".parse().unwrap();
    let s = build(&now, Some((start, &before)), LegalForm::Aktiebolag).unwrap();

    let rent = line(&s.income, "Övriga externa kostnader");
    assert_eq!((rent.amount, rent.previous), (0, Some(-300)));
    assert!(!shown(&s.income, "Personalkostnader"));
    // Headings and subtotals stay, even at 0.
    assert!(shown(&s.income, "Finansiella poster"));
    let financial = line(&s.income, "Summa finansiella poster");
    assert_eq!((financial.amount, financial.previous), (0, Some(0)));
    let net = line(&s.income, "Nettoomsättning");
    assert_eq!((net.amount, net.previous), (1_000, Some(1_000)));
    assert_eq!(s.previous_fiscal_year_start, Some(start));
    assert_eq!(s.previous_difference, Some(0));
}

#[test]
fn without_a_previous_year_nothing_has_a_comparison() {
    let s = ab(&trading());
    assert!(
        s.income
            .iter()
            .chain(&s.balance)
            .all(|l| l.previous.is_none())
    );
    assert_eq!(s.previous_difference, None);
    assert_eq!(s.previous_fiscal_year_start, None);
}

#[test]
fn an_earlier_open_year_leaves_a_difference() {
    // Year 2 while year 1 (a 700 profit) is open: the cash came in, but
    // the result never reached equity.
    let rows = [row(1930, 10_700, 0, 0), row(2081, -10_000, 0, 0)];
    let s = ab(&rows);
    assert_eq!(amount(&s.balance, "Summa tillgångar"), 10_700);
    assert_eq!(amount(&s.balance, "Summa eget kapital och skulder"), 10_000);
    assert_eq!(s.difference, 700);
}

#[test]
fn liabilities_show_credit_balances_as_positive() {
    let rows = [
        row(1930, 0, 500, 0),
        row(2440, 0, 0, 200),
        row(2350, 0, 0, 300),
    ];
    let s = ab(&rows);
    assert_eq!(amount(&s.balance, "Leverantörsskulder"), 200);
    assert_eq!(amount(&s.balance, "Summa kortfristiga skulder"), 200);
    assert_eq!(amount(&s.balance, "Långfristiga skulder"), 300);
    assert_eq!(s.difference, 0);
}

#[test]
fn an_overflow_is_an_error_not_a_panic() {
    let one_row = [row(1930, i64::MAX, 1, 0)];
    assert!(matches!(
        build(&one_row, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
    let two_rows = [row(1930, i64::MAX, 0, 0), row(1910, 1, 0, 0)];
    assert!(matches!(
        build(&two_rows, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
    let negated = [row(2081, i64::MIN, 0, 0)];
    assert!(matches!(
        build(&negated, None, LegalForm::Aktiebolag),
        Err(Error::Overflow)
    ));
}
