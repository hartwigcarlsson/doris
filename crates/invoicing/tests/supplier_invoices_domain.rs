use doris_company::domain::AccountingMethod;
use doris_invoicing::domain::{DomainError, Party, PartyName, SupplierDetails, SupplierForm};
use doris_invoicing::supplier_invoices::*;
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(number: u32) -> AccountNumber {
    AccountNumber::parse(number).unwrap()
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

fn supplier(active: bool) -> Party<SupplierDetails> {
    let details = SupplierDetails::parse(&SupplierForm {
        name: "Lev AB",
        org_nr: "556036-0793",
        vat_number: "",
        street: "",
        postal_code: "",
        city: "",
        email: "",
        bankgiro: "5050-1055",
        plusgiro: "",
        iban: "",
        bic: "",
    })
    .unwrap();
    Party {
        number: 3,
        details,
        active,
    }
}

fn new_invoice(lines: Vec<InvoiceLine>) -> NewSupplierInvoice<'static> {
    NewSupplierInvoice {
        invoice_number: "F-4711",
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines,
        vat: None,
    }
}

fn snapshot() -> SupplierSnapshot {
    SupplierSnapshot::of(&supplier(true)).unwrap()
}

fn registration() -> Registration {
    Registration::new(snapshot(), &new_invoice(vec![line(5410, 80_000, 25)])).unwrap()
}

fn voucher(number: u32) -> VoucherRef {
    VoucherRef {
        fiscal_year_start: d("2026-01-01"),
        number,
    }
}

fn registered(number: u32, invoice: Registration) -> SupplierInvoiceEvent {
    SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments: vec![],
        voucher: Some(voucher(number)),
    }
}

#[test]
fn invoice_numbers_and_references_are_trimmed_and_bounded() {
    assert_eq!(InvoiceNumber::parse(" F-4711 ").unwrap().as_str(), "F-4711");
    for bad in ["", "  ", &"x".repeat(51)] {
        assert_eq!(
            InvoiceNumber::parse(bad),
            Err(DomainError::InvalidInvoiceNumber)
        );
    }
    assert_eq!(
        PaymentReference::parse(" 12345 ").unwrap().as_str(),
        "12345"
    );
    assert_eq!(
        PaymentReference::parse(&"1".repeat(51)),
        Err(DomainError::InvalidReference)
    );
}

#[test]
fn payment_accounts_are_19xx() {
    assert_eq!(payment_account(1930).unwrap().get(), 1930);
    for bad in [1899, 2000, 2440, 5] {
        assert_eq!(
            payment_account(bad),
            Err(DomainError::InvalidPaymentAccount),
            "{bad}"
        );
    }
}

#[test]
fn reasons_are_1_to_200_characters() {
    assert_eq!(reason(" Dubbel "), Ok("Dubbel".to_owned()));
    for bad in ["", " ", &"å".repeat(201)] {
        assert_eq!(reason(bad), Err(DomainError::InvalidReason));
    }
}

#[test]
fn a_registration_works_out_vat_and_total_and_copies_the_supplier() {
    let r = registration();
    assert_eq!((r.vat, r.total), (20_000, 100_000));
    assert_eq!(r.supplier.number, 3);
    assert_eq!(r.supplier.name.as_str(), "Lev AB");
    assert_eq!(
        r.supplier.bankgiro.as_ref().unwrap().formatted(),
        "5050-1055"
    );
    assert_eq!(r.reference, None);
    let with_vat = Registration::new(
        snapshot(),
        &NewSupplierInvoice {
            vat: Some(20_050),
            reference: "OCR 123",
            ..new_invoice(vec![line(5410, 80_000, 25)])
        },
    )
    .unwrap();
    assert_eq!((with_vat.vat, with_vat.total), (20_050, 100_050));
    assert_eq!(with_vat.reference.unwrap().as_str(), "OCR 123");
}

#[test]
fn an_inactive_supplier_cannot_be_invoiced() {
    assert_eq!(
        SupplierSnapshot::of(&supplier(false)),
        Err(DomainError::SupplierInactive)
    );
}

#[test]
fn each_bad_registration_field_gives_its_own_error() {
    let bad = |new: NewSupplierInvoice| Registration::new(snapshot(), &new).unwrap_err();
    let ok = || new_invoice(vec![line(5410, 80_000, 25)]);
    assert_eq!(
        bad(NewSupplierInvoice {
            invoice_number: "",
            ..ok()
        }),
        DomainError::InvalidInvoiceNumber
    );
    assert_eq!(
        bad(NewSupplierInvoice {
            due_date: d("2026-02-28"),
            ..ok()
        }),
        DomainError::InvalidDueDate
    );
    let long = "1".repeat(51);
    assert_eq!(
        bad(NewSupplierInvoice {
            reference: &long,
            ..ok()
        }),
        DomainError::InvalidReference
    );
    assert_eq!(bad(new_invoice(vec![])), DomainError::InvalidInvoiceLines);
    assert_eq!(
        bad(NewSupplierInvoice {
            vat: Some(25_000),
            ..ok()
        }),
        DomainError::InvalidVatAmount
    );
    assert!(
        Registration::new(
            snapshot(),
            &NewSupplierInvoice {
                due_date: d("2026-03-01"),
                ..ok()
            }
        )
        .is_ok()
    );
}

#[test]
fn numbers_run_1_to_n_and_duplicates_are_refused_until_cancelled() {
    let mut state = SupplierInvoices::default();
    assert_eq!(register(&state, &registration()), Ok(1));
    state.apply(registered(1, registration()));
    assert_eq!(
        register(&state, &registration()),
        Err(DomainError::DuplicateSupplierInvoice)
    );
    let mut other_supplier = registration();
    other_supplier.supplier.number = 4;
    assert_eq!(register(&state, &other_supplier), Ok(2));
    state.apply(SupplierInvoiceEvent::SupplierInvoiceCancelled {
        number: 1,
        reason: "Dubbel".into(),
        voucher: Some(voucher(2)),
    });
    assert_eq!(register(&state, &registration()), Ok(2));
}

#[test]
fn an_invoice_is_paid_once_and_a_reversed_payment_makes_it_unpaid() {
    let mut state = SupplierInvoices::from_events([registered(1, registration())]);
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(
        paid(&state, 1).unwrap_err(),
        DomainError::SupplierInvoiceNotPaid
    );
    state.apply(SupplierInvoiceEvent::SupplierInvoicePaid {
        number: 1,
        date: d("2026-03-20"),
        account: a(1930),
        voucher: voucher(2),
    });
    assert_eq!(
        unpaid(&state, 1).unwrap_err(),
        DomainError::SupplierInvoicePaid
    );
    assert_eq!(paid(&state, 1).unwrap().1, voucher(2));
    assert_eq!(state.get(1).unwrap().status_code(), "paid");
    state.apply(SupplierInvoiceEvent::SupplierInvoicePaymentReversed {
        number: 1,
        reason: "Fel".into(),
        voucher: voucher(3),
    });
    assert!(unpaid(&state, 1).is_ok());
    assert_eq!(
        state.get(1).unwrap().vouchers,
        [voucher(1), voucher(2), voucher(3)]
    );
    assert_eq!(state.get(1).unwrap().registration_voucher, Some(voucher(1)));
}

#[test]
fn a_cancelled_invoice_can_be_neither_paid_nor_reversed_and_unknown_ones_are_not_found() {
    let mut state = SupplierInvoices::from_events([registered(1, registration())]);
    state.apply(SupplierInvoiceEvent::SupplierInvoiceCancelled {
        number: 1,
        reason: "Dubbel".into(),
        voucher: None,
    });
    assert_eq!(
        unpaid(&state, 1).unwrap_err(),
        DomainError::SupplierInvoiceCancelled
    );
    assert_eq!(
        paid(&state, 1).unwrap_err(),
        DomainError::SupplierInvoiceCancelled
    );
    assert_eq!(state.get(1).unwrap().status_code(), "cancelled");
    assert_eq!(
        unpaid(&state, 9).unwrap_err(),
        DomainError::SupplierInvoiceNotFound
    );
    assert_eq!(
        paid(&state, 9).unwrap_err(),
        DomainError::SupplierInvoiceNotFound
    );
}

#[test]
fn registration_books_cost_and_vat_against_2440() {
    let r = Registration::new(
        snapshot(),
        &new_invoice(vec![line(5410, 80_000, 25), line(6110, 10_000, 0)]),
    )
    .unwrap();
    assert_eq!(
        registration_lines(&r),
        [
            debit(5410, 80_000),
            debit(6110, 10_000),
            debit(2640, 20_000),
            credit(2440, 110_000)
        ]
    );
}

#[test]
fn an_invoice_without_vat_has_no_2640_line() {
    let r = Registration::new(snapshot(), &new_invoice(vec![line(6110, 10_000, 0)])).unwrap();
    assert_eq!(
        registration_lines(&r),
        [debit(6110, 10_000), credit(2440, 10_000)]
    );
}

#[test]
fn payment_books_2440_under_faktureringsmetoden_and_the_cost_under_kontantmetoden() {
    let r = registration();
    assert_eq!(
        payment_lines(&r, AccountingMethod::Invoice, a(1930)),
        [debit(2440, 100_000), credit(1930, 100_000)]
    );
    assert_eq!(
        payment_lines(&r, AccountingMethod::Cash, a(1930)),
        [
            debit(5410, 80_000),
            debit(2640, 20_000),
            credit(1930, 100_000)
        ]
    );
}

#[test]
fn the_voucher_text_names_the_invoice_and_fits_200_characters() {
    assert_eq!(
        text(12, &registration()),
        "Leverantörsfaktura 12, Lev AB (F-4711)"
    );
    let mut long = registration();
    long.supplier.name = PartyName::parse(&"å".repeat(200)).unwrap();
    assert_eq!(text(12, &long).chars().count(), 200);
}

#[test]
fn a_correction_is_dated_today_or_the_fiscal_years_last_day() {
    assert_eq!(
        correction_date(d("2026-12-31"), d("2026-05-01")),
        d("2026-05-01")
    );
    assert_eq!(
        correction_date(d("2026-12-31"), d("2027-01-10")),
        d("2026-12-31")
    );
}

#[test]
fn stored_events_name_the_supplier_invoice() {
    let event = registered(1, registration());
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "SupplierInvoiceRegistered");
    assert_eq!(json["invoice"]["lines"][0]["vat_rate"], 25);
    assert_eq!(json["voucher"]["number"], 1);
    assert_eq!(
        serde_json::from_value::<SupplierInvoiceEvent>(json).unwrap(),
        event
    );
}
