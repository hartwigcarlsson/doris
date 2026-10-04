use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_invoicing::customer_invoices::NewCustomerInvoice;
use doris_invoicing::domain::{CustomerForm, DomainError};
use doris_invoicing::invoices::Status;
use doris_invoicing::vat::InvoiceLine;
use doris_invoicing::{
    Error, add_customer, cancel_customer_invoice, customer_invoice_attachment,
    list_customer_invoices, pay_customer_invoice, rebuild_projections, register_customer_invoice,
    reverse_customer_invoice_payment, set_customer_active,
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
    add_customer(
        pool,
        id,
        owner,
        &CustomerForm {
            name: "Kund AB",
            org_nr: "556036-0793",
            vat_number: "",
            street: "",
            postal_code: "",
            city: "Stockholm",
            email: "",
            payment_terms: 30,
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

fn invoice(number: &str) -> NewCustomerInvoice<'_> {
    NewCustomerInvoice {
        invoice_number: number,
        invoice_date: d("2026-03-01"),
        due_date: d("2026-03-31"),
        reference: "",
        lines: vec![InvoiceLine::new(3001, 80_000, 25).unwrap()],
    }
}

fn pdf(name: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: format!("%PDF-1.7\n{name}").into_bytes(),
    }
}

async fn register(pool: &SqlitePool, id: Uuid, anna: Uuid, number: &str) -> Result<u32, Error> {
    register_customer_invoice(
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
    pay_customer_invoice(pool, id, anna, number, d("2026-03-20"), 1930, d(TODAY)).await
}

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
async fn faktureringsmetoden_books_the_invoice_and_the_payment() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;

    assert_eq!(register(&pool, id, anna, "1017").await.unwrap(), 1);
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked[0].text, "Kundfaktura 1017, Kund AB");
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 1510).await, 100_000);
    assert_eq!(balance(&pool, id, anna, 2611).await, -20_000);
    assert_eq!(balance(&pool, id, anna, 3001).await, -80_000);

    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
    assert_eq!(balance(&pool, id, anna, 1930).await, 100_000);
    let (listed, next) = list_customer_invoices(&pool, id, anna).await.unwrap();
    assert!(matches!(listed[0].status, Status::Paid { .. }));
    assert_eq!(next, "1018");
}

#[tokio::test]
async fn kontantmetoden_books_only_the_payment_with_the_underlag() {
    let (pool, anna, id) = setup(AccountingMethod::Cash).await;
    register(&pool, id, anna, "1").await.unwrap();
    assert!(vouchers(&pool, id, anna).await.is_empty());

    pay(&pool, id, anna, 1).await.unwrap();
    let booked = vouchers(&pool, id, anna).await;
    assert_eq!(booked.len(), 1);
    assert_eq!(booked[0].attachments.len(), 1);
    assert_eq!(balance(&pool, id, anna, 1930).await, 100_000);
    assert_eq!(balance(&pool, id, anna, 2611).await, -20_000);
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
}

#[tokio::test]
async fn a_cancelled_invoice_number_is_never_reused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1017").await.unwrap();
    cancel_customer_invoice(&pool, id, anna, 1, "Fel kund", d(TODAY))
        .await
        .unwrap();
    assert_eq!(vouchers(&pool, id, anna).await[1].corrects, Some(1));
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);

    let err = register(&pool, id, anna, "1017").await.unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::DuplicateCustomerInvoice)
    ));
    let (listed, next) = list_customer_invoices(&pool, id, anna).await.unwrap();
    assert_eq!(listed[0].status, Status::Cancelled);
    assert_eq!(next, "1018");
}

#[tokio::test]
async fn a_payment_is_reversed_and_paid_again() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    reverse_customer_invoice_payment(&pool, id, anna, 1, "Fel konto", d(TODAY))
        .await
        .unwrap();
    assert_eq!(
        list_customer_invoices(&pool, id, anna).await.unwrap().0[0].status,
        Status::Unpaid
    );
    assert_eq!(balance(&pool, id, anna, 1510).await, 100_000);
    pay(&pool, id, anna, 1).await.unwrap();
    assert_eq!(balance(&pool, id, anna, 1510).await, 0);
}

#[tokio::test]
async fn hand_corrections_are_followed() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    doris_ledger::correct_voucher(&pool, id, anna, d("2026-01-01"), 1, d(TODAY), d(TODAY))
        .await
        .unwrap();
    cancel_customer_invoice(&pool, id, anna, 1, "Rättad i grundboken", d(TODAY))
        .await
        .unwrap();
    assert_eq!(vouchers(&pool, id, anna).await.len(), 2);
    assert_eq!(
        list_customer_invoices(&pool, id, anna).await.unwrap().0[0].status,
        Status::Cancelled
    );
}

#[tokio::test]
async fn wrong_transitions_and_inputs_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    let err = register_customer_invoice(&pool, id, anna, 9, invoice("1"), vec![], d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerNotFound)));
    let future = NewCustomerInvoice {
        invoice_date: d("2026-10-03"),
        due_date: d("2026-11-03"),
        ..invoice("2")
    };
    let err = register_customer_invoice(&pool, id, anna, 1, future, vec![], d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::InvoiceDateInFuture)
    ));
    let err = register_customer_invoice(
        &pool,
        id,
        anna,
        1,
        invoice("3"),
        vec![pdf("a.pdf"), pdf("a.pdf")],
        d(TODAY),
    )
    .await
    .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::DuplicateAttachment);

    register(&pool, id, anna, "1").await.unwrap();
    let err = reverse_customer_invoice_payment(&pool, id, anna, 1, "Fel", d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::CustomerInvoiceNotPaid)
    ));
    pay(&pool, id, anna, 1).await.unwrap();
    let err = cancel_customer_invoice(&pool, id, anna, 1, "Fel", d(TODAY))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::CustomerInvoicePaid)
    ));
    let err = pay(&pool, id, anna, 9).await.unwrap_err();
    assert!(matches!(
        err,
        Error::Domain(DomainError::CustomerInvoiceNotFound)
    ));

    set_customer_active(&pool, id, anna, 1, false)
        .await
        .unwrap();
    let err = register(&pool, id, anna, "4").await.unwrap_err();
    assert!(matches!(err, Error::Domain(DomainError::CustomerInactive)));
}

#[tokio::test]
async fn a_rejected_registration_leaves_nothing_and_uses_up_no_number() {
    for method in [AccountingMethod::Invoice, AccountingMethod::Cash] {
        let (pool, anna, id) = setup(method).await;
        doris_ledger::set_account_active(&pool, id, anna, 3001, false)
            .await
            .unwrap();
        let err = register(&pool, id, anna, "1").await.unwrap_err();
        assert_eq!(
            ledger_error(err),
            LedgerError::AccountInactive,
            "{method:?}"
        );
        assert!(
            list_customer_invoices(&pool, id, anna)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        doris_ledger::set_account_active(&pool, id, anna, 3001, true)
            .await
            .unwrap();
        assert_eq!(register(&pool, id, anna, "1").await.unwrap(), 1);
    }
}

#[tokio::test]
async fn a_closed_year_takes_no_invoice() {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2025-01-01", AccountingMethod::Invoice).await;
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), d(TODAY))
        .await
        .unwrap();
    let old = NewCustomerInvoice {
        invoice_date: d("2025-06-01"),
        due_date: d("2025-07-01"),
        ..invoice("1")
    };
    let err = register_customer_invoice(&pool, id, anna, 1, old, vec![], d(TODAY))
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::FiscalYearClosed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_registrations_of_the_same_number_give_exactly_one() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("customers.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();
    let anna = Uuid::new_v4();
    let id = company(&pool, anna, "2026-01-01", AccountingMethod::Invoice).await;
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { register(&pool, id, anna, "1017").await })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(Error::Domain(DomainError::DuplicateCustomerInvoice)) => {}
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(ok, 1);
}

#[tokio::test]
async fn the_projection_rebuilds_from_the_events() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    register(&pool, id, anna, "1").await.unwrap();
    pay(&pool, id, anna, 1).await.unwrap();
    register(&pool, id, anna, "2").await.unwrap();
    cancel_customer_invoice(&pool, id, anna, 2, "Fel", d(TODAY))
        .await
        .unwrap();
    let sql = "SELECT company_id || number || customer_number || invoice_number || status || details FROM customer_invoices ORDER BY number";
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
    register(&pool, id, anna, "1").await.unwrap();
    let sha = list_customer_invoices(&pool, id, anna).await.unwrap().0[0].attachments[0]
        .sha256
        .clone();
    let (_, data) = customer_invoice_attachment(&pool, id, anna, 1, &sha)
        .await
        .unwrap();
    assert_eq!(data, pdf("faktura.pdf").data);
    let err = customer_invoice_attachment(&pool, id, anna, 2, &sha)
        .await
        .unwrap_err();
    assert_eq!(ledger_error(err), LedgerError::AttachmentNotFound);
    let err = customer_invoice_attachment(&pool, id, Uuid::new_v4(), 1, &sha)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound));
}

#[tokio::test]
async fn an_empty_register_proposes_number_1_and_strangers_are_refused() {
    let (pool, anna, id) = setup(AccountingMethod::Invoice).await;
    assert_eq!(
        list_customer_invoices(&pool, id, anna).await.unwrap().1,
        "1"
    );
    assert!(matches!(
        list_customer_invoices(&pool, id, Uuid::new_v4()).await,
        Err(Error::NotFound)
    ));
}
