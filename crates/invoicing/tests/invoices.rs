use doris_invoicing::domain::DomainError;
use doris_invoicing::invoices::{self, InvoiceKind, Side, Status};
use doris_invoicing::vat::InvoiceLine;
use doris_ledger::VoucherRef;
use doris_ledger::domain::{AccountNumber, VoucherLine};
use jiff::civil::Date;
use serde_json::json;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn a(n: u32) -> AccountNumber {
    AccountNumber::parse(n).unwrap()
}

struct Kind;
impl InvoiceKind for Kind {
    const PAID: DomainError = DomainError::SupplierInvoicePaid;
    const NOT_PAID: DomainError = DomainError::SupplierInvoiceNotPaid;
    const CANCELLED: DomainError = DomainError::SupplierInvoiceCancelled;
}

fn paid() -> Status {
    Status::Paid {
        date: d("2026-03-20"),
        account: a(1930),
        voucher: VoucherRef {
            fiscal_year_start: d("2026-01-01"),
            number: 2,
        },
    }
}

#[test]
fn the_status_is_stored_as_before() {
    assert_eq!(
        serde_json::to_value(Status::Unpaid).unwrap(),
        json!({"status": "unpaid"})
    );
    assert_eq!(
        serde_json::to_value(Status::Cancelled).unwrap(),
        json!({"status": "cancelled"})
    );
    assert_eq!(
        serde_json::to_value(paid()).unwrap(),
        json!({"status": "paid", "date": "2026-03-20", "account": 1930,
               "voucher": {"fiscal_year_start": "2026-01-01", "number": 2}})
    );
}

#[test]
fn only_unpaid_invoices_are_paid_or_cancelled_and_only_paid_ones_reversed() {
    assert_eq!(invoices::check_unpaid::<Kind>(&Status::Unpaid), Ok(()));
    assert_eq!(
        invoices::check_unpaid::<Kind>(&paid()),
        Err(DomainError::SupplierInvoicePaid)
    );
    assert_eq!(
        invoices::check_unpaid::<Kind>(&Status::Cancelled),
        Err(DomainError::SupplierInvoiceCancelled)
    );
    assert_eq!(invoices::check_paid::<Kind>(&paid()).unwrap().number, 2);
    assert_eq!(
        invoices::check_paid::<Kind>(&Status::Unpaid),
        Err(DomainError::SupplierInvoiceNotPaid)
    );
    assert_eq!(
        invoices::check_paid::<Kind>(&Status::Cancelled),
        Err(DomainError::SupplierInvoiceCancelled)
    );
}

#[test]
fn posting_puts_lines_and_vat_on_one_side() {
    let lines = [
        InvoiceLine::new(3001, 80_000, 25).unwrap(),
        InvoiceLine::new(3004, 5_000, 0).unwrap(),
    ];
    assert_eq!(
        invoices::posting(&lines, &[(a(2611), 20_000)], Side::Credit),
        [
            VoucherLine {
                account: a(3001),
                debit: 0,
                credit: 80_000
            },
            VoucherLine {
                account: a(3004),
                debit: 0,
                credit: 5_000
            },
            VoucherLine {
                account: a(2611),
                debit: 0,
                credit: 20_000
            },
        ]
    );
    assert_eq!(
        invoices::entry(a(1510), 105_000, Side::Debit),
        VoucherLine {
            account: a(1510),
            debit: 105_000,
            credit: 0
        }
    );
}

#[test]
fn voucher_texts_are_cut_to_200_characters() {
    assert_eq!(invoices::voucher_text("Kort".into()), "Kort");
    assert_eq!(invoices::voucher_text("å".repeat(250)).chars().count(), 200);
}
