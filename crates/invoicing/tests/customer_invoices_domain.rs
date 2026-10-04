use doris_company::domain::AccountingMethod;
use doris_invoicing::customer_invoices::*;
use doris_invoicing::domain::{CustomerDetails, CustomerForm, DomainError, Party, PartyName};
use doris_invoicing::invoices::Status;
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(n: u32) -> AccountNumber {
    AccountNumber::parse(n).unwrap()
}

fn debit(account: u32, ore: i64) -> VoucherLine {
    VoucherLine {
        account: a(account),
        debit: ore,
        credit: 0,
    }
}

fn credit(account: u32, ore: i64) -> VoucherLine {
    VoucherLine {
        account: a(account),
        debit: 0,
        credit: ore,
    }
}

fn line(account: u32, net: i64, rate: u32) -> InvoiceLine {
    InvoiceLine::new(account, net, rate).unwrap()
}

fn customer(active: bool) -> Party<CustomerDetails> {
    let details = CustomerDetails::parse(&CustomerForm {
        name: "Kund AB",
        org_nr: "556036-0793",
        vat_number: "",
        street: "Storgatan 1",
        postal_code: "111 22",
        city: "Stockholm",
        email: "",
        payment_terms: 30,
    })
    .unwrap();
    Party {
        number: 7,
        details,
        active,
    }
}

fn new_invoice(lines: Vec<InvoiceLine>) -> NewCustomerInvoice<'static> {
    NewCustomerInvoice {
        invoice_number: "1017",
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines,
    }
}

fn registration(lines: Vec<InvoiceLine>) -> CustomerRegistration {
    CustomerRegistration::new(
        CustomerSnapshot::of(&customer(true)).unwrap(),
        &new_invoice(lines),
    )
    .unwrap()
}

fn voucher(number: u32) -> VoucherRef {
    VoucherRef {
        fiscal_year_start: d("2026-01-01"),
        number,
    }
}

fn registered(number: u32, invoice: CustomerRegistration) -> CustomerInvoiceEvent {
    CustomerInvoiceEvent::CustomerInvoiceRegistered {
        number,
        invoice,
        attachments: vec![],
        voucher: Some(voucher(number)),
    }
}

#[test]
fn output_vat_goes_to_2611_2621_and_2631() {
    use doris_invoicing::vat::VatRate;
    assert_eq!(
        output_vat_account(VatRate::parse(25).unwrap()),
        Some(a(2611))
    );
    assert_eq!(
        output_vat_account(VatRate::parse(12).unwrap()),
        Some(a(2621))
    );
    assert_eq!(
        output_vat_account(VatRate::parse(6).unwrap()),
        Some(a(2631))
    );
    assert_eq!(output_vat_account(VatRate::parse(0).unwrap()), None);
}

#[test]
fn a_registration_copies_the_customer_and_works_out_vat_per_rate() {
    let r = registration(vec![line(3001, 80_000, 25), line(3002, 10_000, 12)]);
    assert_eq!(r.customer.number, 7);
    assert_eq!(r.customer.name.as_str(), "Kund AB");
    assert_eq!(r.customer.address.city.as_deref(), Some("Stockholm"));
    let vat: Vec<_> = r
        .vat
        .iter()
        .map(|v| (v.vat_rate.percent(), v.amount))
        .collect();
    assert_eq!(vat, [(25, 20_000), (12, 1_200)]);
    assert_eq!((r.vat_total(), r.total), (21_200, 111_200));
}

#[test]
fn an_inactive_customer_cannot_be_invoiced() {
    assert_eq!(
        CustomerSnapshot::of(&customer(false)),
        Err(DomainError::CustomerInactive)
    );
}

#[test]
fn each_bad_registration_field_gives_its_own_error() {
    let snapshot = || CustomerSnapshot::of(&customer(true)).unwrap();
    let ok = || new_invoice(vec![line(3001, 100, 25)]);
    let bad = |new: NewCustomerInvoice| CustomerRegistration::new(snapshot(), &new).unwrap_err();
    assert_eq!(
        bad(NewCustomerInvoice {
            invoice_number: " ",
            ..ok()
        }),
        DomainError::InvalidInvoiceNumber
    );
    assert_eq!(
        bad(NewCustomerInvoice {
            due_date: d("2026-02-28"),
            ..ok()
        }),
        DomainError::InvalidDueDate
    );
    let long = "1".repeat(51);
    assert_eq!(
        bad(NewCustomerInvoice {
            reference: &long,
            ..ok()
        }),
        DomainError::InvalidReference
    );
    assert_eq!(bad(new_invoice(vec![])), DomainError::InvalidInvoiceLines);
    assert_eq!(
        InvoiceLine::new(1510, 100, 25),
        Err(DomainError::InvalidInvoiceAccount)
    );
}

#[test]
fn a_due_date_on_the_invoice_date_is_accepted() {
    let snapshot = CustomerSnapshot::of(&customer(true)).unwrap();
    let new = NewCustomerInvoice {
        due_date: d("2026-03-01"),
        ..new_invoice(vec![line(3001, 100, 25)])
    };
    assert!(CustomerRegistration::new(snapshot, &new).is_ok());
}

#[test]
fn faktureringsmetoden_books_1510_against_income_and_output_vat() {
    let r = registration(vec![line(3001, 80_000, 25), line(3002, 10_000, 12)]);
    assert_eq!(
        registration_lines(&r),
        [
            debit(1510, 111_200),
            credit(3001, 80_000),
            credit(3002, 10_000),
            credit(2611, 20_000),
            credit(2621, 1_200)
        ]
    );
    assert_eq!(
        payment_lines(&r, AccountingMethod::Invoice, a(1930)),
        [debit(1930, 111_200), credit(1510, 111_200)]
    );
}

#[test]
fn kontantmetoden_books_income_and_output_vat_at_payment() {
    let r = registration(vec![line(3001, 80_000, 25)]);
    assert_eq!(
        payment_lines(&r, AccountingMethod::Cash, a(1930)),
        [
            debit(1930, 100_000),
            credit(3001, 80_000),
            credit(2611, 20_000)
        ]
    );
}

#[test]
fn mixed_rates_book_vat_per_rate_and_none_for_0_percent() {
    let r = registration(vec![
        line(3004, 5_000, 0),
        line(3003, 1_000, 6),
        line(3001, 33, 25),
    ]);
    assert_eq!(
        registration_lines(&r),
        [
            debit(1510, 6_101),
            credit(3004, 5_000),
            credit(3003, 1_000),
            credit(3001, 33),
            credit(2611, 8),
            credit(2631, 60)
        ]
    );
    assert_eq!(r.total, 5_000 + 1_000 + 33 + 8 + 60);
}

#[test]
fn the_voucher_text_names_the_invoice_and_fits_200_characters() {
    let mut r = registration(vec![line(3001, 100, 25)]);
    assert_eq!(text(&r), "Kundfaktura 1017, Kund AB");
    r.customer.name = PartyName::parse(&"å".repeat(200)).unwrap();
    assert_eq!(text(&r).chars().count(), 200);
}

#[test]
fn an_invoice_number_is_never_reused_even_after_cancelling() {
    let mut state = CustomerInvoices::default();
    let r = || registration(vec![line(3001, 100, 25)]);
    assert_eq!(register(&state, &r()), Ok(1));
    state.apply(registered(1, r()));
    assert_eq!(
        register(&state, &r()),
        Err(DomainError::DuplicateCustomerInvoice)
    );
    state.apply(CustomerInvoiceEvent::CustomerInvoiceCancelled {
        number: 1,
        reason: "Fel kund".into(),
        voucher: Some(voucher(2)),
    });
    assert_eq!(
        register(&state, &r()),
        Err(DomainError::DuplicateCustomerInvoice)
    );
    let mut other = r();
    other.invoice_number =
        doris_invoicing::supplier_invoices::InvoiceNumber::parse("1018").unwrap();
    assert_eq!(register(&state, &other), Ok(2));
}

#[test]
fn an_invoice_is_paid_once_and_a_reversed_payment_makes_it_unpaid() {
    let mut state =
        CustomerInvoices::from_events([registered(1, registration(vec![line(3001, 100, 25)]))]);
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(
        paid(&state, 1).unwrap_err(),
        DomainError::CustomerInvoiceNotPaid
    );
    state.apply(CustomerInvoiceEvent::CustomerInvoicePaid {
        number: 1,
        date: d("2026-03-20"),
        account: a(1930),
        voucher: voucher(2),
    });
    assert_eq!(
        unpaid(&state, 1).unwrap_err(),
        DomainError::CustomerInvoicePaid
    );
    assert_eq!(paid(&state, 1).unwrap().1, voucher(2));
    assert_eq!(state.get(1).unwrap().status_code(), "paid");
    state.apply(CustomerInvoiceEvent::CustomerInvoicePaymentReversed {
        number: 1,
        reason: "Fel".into(),
        voucher: voucher(3),
    });
    assert_eq!(state.get(1).unwrap().status, Status::Unpaid);
    assert_eq!(
        state.get(1).unwrap().vouchers,
        [voucher(1), voucher(2), voucher(3)]
    );
    assert_eq!(
        unpaid(&state, 9).unwrap_err(),
        DomainError::CustomerInvoiceNotFound
    );
    state.apply(CustomerInvoiceEvent::CustomerInvoiceCancelled {
        number: 1,
        reason: "Fel".into(),
        voucher: None,
    });
    assert_eq!(
        paid(&state, 1).unwrap_err(),
        DomainError::CustomerInvoiceCancelled
    );
}

#[test]
fn the_next_number_follows_the_highest_plain_number() {
    assert_eq!(next_invoice_number([]), "1");
    assert_eq!(next_invoice_number(["1017", "998", "1016"]), "1018");
    assert_eq!(next_invoice_number(["0017"]), "18");
}

#[test]
fn the_next_number_skips_what_isnt_a_plain_number() {
    assert_eq!(next_invoice_number(["2026-17", "A12", " 5", ""]), "1");
    assert_eq!(next_invoice_number(["9", &"9".repeat(30)]), "10");
}

#[test]
fn stored_events_name_the_customer_invoice() {
    let event = registered(1, registration(vec![line(3001, 100, 25)]));
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "CustomerInvoiceRegistered");
    assert_eq!(json["invoice"]["vat"][0]["vat_rate"], 25);
    assert_eq!(
        serde_json::from_value::<CustomerInvoiceEvent>(json).unwrap(),
        event
    );
}
