use doris_invoicing::domain::DomainError;
use doris_invoicing::vat::{self, InvoiceLine, VatRate};

fn line(account: u32, net: i64, rate: u32) -> InvoiceLine {
    InvoiceLine::new(account, net, rate).unwrap()
}

#[test]
fn vat_rates_are_25_12_6_or_0_percent() {
    for percent in [25, 12, 6, 0] {
        assert_eq!(VatRate::parse(percent).unwrap().percent(), percent);
    }
    assert_eq!(VatRate::parse(20), Err(DomainError::InvalidVatRate));
}

#[test]
fn a_line_needs_a_positive_amount_and_a_cost_account() {
    assert_eq!(line(5410, 1, 25).net, 1);
    for net in [0, -100, vat::MAX_LINE_NET + 1] {
        assert_eq!(
            InvoiceLine::new(5410, net, 25),
            Err(DomainError::InvalidInvoiceLines),
            "{net}"
        );
    }
    for account in [2440, 2640, 2611, 2600, 2699, 999, 9000] {
        assert_eq!(
            InvoiceLine::new(account, 100, 25),
            Err(DomainError::InvalidInvoiceAccount),
            "{account}"
        );
    }
    assert_eq!(
        InvoiceLine::new(5410, 100, 20),
        Err(DomainError::InvalidVatRate)
    );
}

#[test]
fn an_invoice_has_1_to_50_lines() {
    assert_eq!(vat::check_lines(&[]), Err(DomainError::InvalidInvoiceLines));
    assert_eq!(vat::check_lines(&vec![line(5410, 1, 25); 50]), Ok(()));
    assert_eq!(
        vat::check_lines(&vec![line(5410, 1, 25); 51]),
        Err(DomainError::InvalidInvoiceLines)
    );
}

#[test]
fn vat_is_rounded_per_rate_not_per_line() {
    // 99 öre at 25 % is 24,75 öre: 25 öre, where per-line rounding gives 24.
    let lines = [line(5410, 33, 25), line(5410, 33, 25), line(5410, 33, 25)];
    assert_eq!(vat::computed(&lines), 25);
    assert_eq!(vat::net(&lines), 99);
    // 12 % of 10 kr, 6 % of 50 öre (3 öre), nothing on 0 %.
    let mixed = [line(5410, 1000, 12), line(4010, 50, 6), line(6110, 700, 0)];
    assert_eq!(vat::computed(&mixed), 123);
}

#[test]
fn a_given_vat_may_differ_by_at_most_one_krona() {
    let lines = [line(5410, 80_000, 25)];
    assert_eq!(vat::check(&lines, None), Ok(20_000));
    assert_eq!(vat::check(&lines, Some(20_100)), Ok(20_100));
    assert_eq!(vat::check(&lines, Some(19_900)), Ok(19_900));
    for bad in [20_101, 19_899] {
        assert_eq!(
            vat::check(&lines, Some(bad)),
            Err(DomainError::InvalidVatAmount)
        );
    }
    assert_eq!(
        vat::check(&[line(6110, 100, 0)], Some(-1)),
        Err(DomainError::InvalidVatAmount)
    );
}
