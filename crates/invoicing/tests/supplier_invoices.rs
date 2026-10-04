use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::domain::{DomainError, SupplierForm};
use doris_invoicing::supplier_invoices::{NewSupplierInvoice, Status};
use doris_invoicing::vat::InvoiceLine;
use doris_invoicing::{
    Error, add_supplier, cancel_supplier_invoice, list_supplier_invoices, pay_supplier_invoice,
    rebuild_projections, register_supplier_invoice, reverse_supplier_invoice_payment,
    set_supplier_active, supplier_invoice_attachment,
};
use doris_ledger::NewAttachment;
use doris_ledger::domain::DomainError as LedgerError;
use jiff::civil::Date;
use sqlx::SqlitePool;
use uuid::Uuid;

const TODAY: &str = "2026-10-02";

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

async fn company(pool: &SqlitePool, owner: Uuid, start: &str, method: AccountingMethod) -> Uuid {
    let year = &start[..4];
    let id = doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: start.parse().unwrap(),
            fiscal_year_end: format!("{year}-12-31").parse().unwrap(),
            accounting_method: method,
        },
    )
    .await
    .unwrap();
    add_supplier(
        pool,
        id,
        owner,
        &SupplierForm {
            name: "Lev AB",
            org_nr: "",
            vat_number: "",
            street: "",
            postal_code: "",
            city: "",
            email: "",
            bankgiro: "5050-1055",
            plusgiro: "",
            iban: "",
            bic: "",
        },
    )
    .await
    .unwrap();
    id
}

async fn setup(method: AccountingMethod) -> (SqlitePool, Uuid, Uuid) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", method).await;
    (pool, anna, id)
}

fn invoice(number: &str) -> NewSupplierInvoice<'_> {
    NewSupplierInvoice {
        invoice_number: number,
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines: vec![InvoiceLine::new(5410, 80_000, 25).unwrap()],
        vat: None,
    }
}

fn pdf(name: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: format!("%PDF-1.7\n{name}").into_bytes(),
    }
}

async fn register(pool: &SqlitePool, id: Uuid, anna: Uuid, number: &str) -> Result<u32, Error> {
    register_supplier_invoice(
        pool,
        id,
        anna,
        1,
        invoice(number),
        vec![pdf("faktura.pdf")],
        d(TODAY),
    )
    .await
}

async fn pay(pool: &SqlitePool, id: Uuid, anna: Uuid, number: u32) -> Result<(), Error> {
    pay_supplier_invoice(pool, id, anna, number, d("2026-03-20"), 1930, d(TODAY)).await
}

/// Utgående balans, debit − credit, of `account` in 2026.
async fn balance(pool: &SqlitePool, id: Uuid, anna: Uuid, account: u32) -> i64 {
    doris_ledger::trial_balance(pool, id, anna, d("2026-01-01"))
        .await
        .unwrap()
        .iter()
        .find(|r| r.account == account)
        .map_or(0, |r| r.opening + r.debit - r.credit)
}

async fn vouchers(pool: &SqlitePool, id: Uuid, anna: Uuid) -> Vec<doris_ledger::domain::Voucher> {
    doris_ledger::list_vouchers(pool, id, anna, d("2026-01-01"))
        .await
        .unwrap()
}

fn ledger_error(err: Error) -> LedgerError {
    match err {
        Error::Ledger(doris_ledger::Error::Domain(e)) => e,
        other => panic!("not a ledger domain error: {other:?}"),
    }
}

#[tokio::test]
async fn faktureringsmetoden_books_the_registration_and_the_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;

    assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 1);
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].text, "Leverantörsfaktura 1, Lev AB (F-4711)");
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 2440).await, -100_000);
    assert_eq!(balance(&pool, id, anna, 2640).await, 20_000);

    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
    assert_eq!(balance(&pool, id, anna, 1930).await, -100_000);
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert!(matches!(listed[0].status, Status::Paid { .. }));
    assert_eq!(listed[0].vouchers.len(), 2);
}

#[tokio::test]
async fn kontantmetoden_books_only_the_payment_with_the_underlag() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;

    register(&pool, id, anna, "F-4711").await.unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].registration_voucher, None);

    pay(&pool, id, anna, 1).await.unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].date, d("2026-03-20"));
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 5410).await, 80_000);
    assert_eq!(balance(&pool, id, anna, 2640).await, 20_000);
    assert_eq!(balance(&pool, id, anna, 1930).await, -100_000);
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
}

#[tokio::test]
async fn cash_method_links_the_underlag_to_each_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    reverse_supplier_invoice_payment(&pool, id, anna, 1, "Fel datum", d(TODAY))
        .await
        .unwrap();
    pay(&pool, id, anna, 1).await.unwrap();

    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 3);
    assert_eq!(booked[2].attachments.len(), 1);
}

#[tokio::test]
async fn cancelling_and_reversing_book_corrections() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();

    cancel_supplier_invoice(&pool, id, anna, 1, "Dubbelregistrerad", d(TODAY))
        .await
        .unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked[1].corrects, Some(1));
    assert_eq!(booked[1].date, d(TODAY));
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
    let err = pay(&pool, id, anna, 1).await.unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::SupplierInvoiceCancelled)
    ));

    assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 2);
    pay(&pool, id, anna, 2).await.unwrap();
    reverse_supplier_invoice_payment(&pool, id, anna, 2, "Fel konto", d(TODAY))
        .await
        .unwrap();
    let listed = list_supplier_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].number, 2);
    assert_eq!(listed[0].status, Status::Unpaid);
    assert_eq!(balance(&pool, id, anna, 1930).await, 0);
    pay(&pool, id, anna, 2).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 2440).await, 0);
}

#[tokio::test]
async fn cancelling_under_kontantmetoden_books_nothing() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    cancel_supplier_invoice(&pool, id, anna, 1, "Fel leverantör", d(TODAY))
        .await
        .unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());
    assert_eq!(
        list_supplier_invoices(&pool, id, anna).await.unwrap()[0].status,
        Status::Cancelled
    );
}

#[tokio::test]
async fn wrong_transitions_and_reasons_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    let err = reverse_supplier_invoice_payment(&pool, id, anna, 1, "Fel", d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::SupplierInvoiceNotPaid)
    ));
    let err = cancel_supplier_invoice(&pool, id, anna, 1, " ", d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::InvalidReason)));
    pay(&pool, id, anna, 1).await.unwrap();
    let err = cancel_supplier_invoice(&pool, id, anna, 1, "Fel", d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::SupplierInvoicePaid)
    ));
    let err = pay(&pool, id, anna, 9).await.unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::SupplierInvoiceNotFound)
    ));
    let err = pay_supplier_invoice(&pool, id, anna, 1, d("2026-03-20"), 2440, d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::InvalidPaymentAccount)
    ));
}

#[tokio::test]
async fn a_rejected_registration_leaves_no_invoice_and_uses_up_no_number() {
    for method in [AccountingMethod::Invoice, AccountingMethod::Cash] {
        let (pool, anna, id) = setup(method).await;
        doris_ledger::set_account_active(&pool, id, anna, 5410, false)
            .await
            .unwrap();

        let err = register(&pool, id, anna, "F-4711").await.unwrap_err();
        assert_eq!(
            ledger_error(err),
            LedgerError::AccountInactive,
            "{method:?}"
        );
        assert!(
            list_supplier_invoices(&pool, id, anna)
                .await
                .unwrap()
                .is_empty()
        );

        doris_ledger::set_account_active(&pool, id, anna, 5410, true)
            .await
            .unwrap();
        assert_eq!(register(&pool, id, anna, "F-4711").await.unwrap(), 1);
        if method == AccountingMethod::Invoice {
            assert_eq!(vouchers(&pool, id, anna).await[0].number, 1);
        }
    }
}

#[tokio::test]
async fn the_supplier_must_exist_and_be_active_and_the_date_not_in_the_future() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    let err = register_supplier_invoice(&pool, id, anna, 9, invoice("F-1"), vec![], d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierNotFound)));
    let future = NewSupplierInvoice {
        invoice_date: d("2026-10-03"),
        due_date: d("2026-11-03"),
        ..invoice("F-2")
    };
    let err = register_supplier_invoice(&pool, id, anna, 1, future, vec![], d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::InvoiceDateInFuture)
    ));
    set_supplier_active(&pool, id, anna, 1, false)
        .await
        .unwrap();
    let err = register(&pool, id, anna, "F-3").await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::SupplierInactive)));
}

#[tokio::test]
async fn the_same_file_twice_is_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    let err = register_supplier_invoice(
        &pool,
        id,
        anna,
        1,
        invoice("F-4711"),
        vec![pdf("a.pdf"), pdf("a.pdf")],
        d(TODAY),
    )
    .await
    .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::DuplicateAttachment);
}

#[tokio::test]
async fn a_closed_year_takes_no_invoice() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), d(TODAY))
        .await
        .unwrap();
    let old = NewSupplierInvoice {
        invoice_date: d("2025-06-01"),
        due_date: d("2025-07-01"),
        ..invoice("F-1")
    };
    let err = register_supplier_invoice(&pool, id, anna, 1, old, vec![], d(TODAY))
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::FiscalYearClosed);
}

#[tokio::test]
async fn a_correction_after_the_year_ended_is_dated_its_last_day() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    let old = NewSupplierInvoice {
        invoice_date: d("2025-06-01"),
        due_date: d("2025-07-01"),
        ..invoice("F-1")
    };
    register_supplier_invoice(&pool, id, anna, 1, old, vec![], d(TODAY))
        .await
        .unwrap();

    cancel_supplier_invoice(&pool, id, anna, 1, "Dubbel", d(TODAY))
        .await
        .unwrap();

    let booked = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    assert_eq!(booked[1].date, d("2025-12-31"));
}

#[tokio::test]
async fn a_hand_corrected_registration_cannot_be_cancelled() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    doris_ledger::correct_voucher(&pool, id, anna, d("2026-01-01"), 1, d(TODAY), d(TODAY))
        .await
        .unwrap();

    let err = cancel_supplier_invoice(&pool, id, anna, 1, "Dubbel", d(TODAY))
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AlreadyCorrected);
    assert_eq!(
        list_supplier_invoices(&pool, id, anna).await.unwrap()[0].status,
        Status::Unpaid
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_registrations_of_the_same_invoice_give_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("invoices.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", AccountingMethod::Invoice).await;

    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { register(&pool, id, anna, "F-4711").await })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(Error::Domain(DomainError::DuplicateSupplierInvoice)) => {}
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(ok, 1);
    assert_eq!(
        list_supplier_invoices(&pool, id, anna).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn the_projection_rebuilds_from_the_events() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "F-1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    register(&pool, id, anna, "F-2").await.unwrap();
    cancel_supplier_invoice(&pool, id, anna, 2, "Fel", d(TODAY))
        .await
        .unwrap();
    let sql = "SELECT company_id || number || supplier_number || invoice_number || status || details FROM supplier_invoices ORDER BY number";
    let rows = || async {
        sqlx::query_scalar::<_, String>(sql)
            .fetch_all(&pool)
            .await
            .unwrap()
    };
    let before = rows().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(rows().await, before);
    assert_eq!(before.len(), 2);
}

#[tokio::test]
async fn an_underlag_is_read_only_through_the_companys_own_invoice() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "F-4711").await.unwrap();
    let sha = list_supplier_invoices(&pool, id, anna).await.unwrap()[0].attachments[0]
        .sha256
        .clone();

    let (attachment, data) = supplier_invoice_attachment(&pool, id, anna, 1, &sha)
        .await
        .unwrap();
    assert_eq!(attachment.file_name.as_str(), "faktura.pdf");
    assert_eq!(data, pdf("faktura.pdf").data);

    let err = supplier_invoice_attachment(&pool, id, anna, 2, &sha)
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AttachmentNotFound);
    let bo = Uuid::new_v4();
    let err = supplier_invoice_attachment(&pool, id, bo, 1, &sha)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound));
}

#[tokio::test]
async fn a_non_member_gets_not_found() {
    let (pool, _anna, id) = setup(AccountingMethod::Invoice).await;
    let bo = Uuid::new_v4();
    assert!(matches!(
        list_supplier_invoices(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        register(&pool, id, bo, "F-1").await,
        Err(Error::NotFound)
    ));
}
