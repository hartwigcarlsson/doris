-- Supplier invoices: the projection of the supplier-invoices-{company}
-- streams. `details` is the invoice as listed (JSON).
CREATE TABLE supplier_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    supplier_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    details         TEXT    NOT NULL,
    PRIMARY KEY (company_id, number)
);

-- One live invoice per supplier and invoice number (dubbelregistrering).
CREATE UNIQUE INDEX supplier_invoices_no_duplicates
    ON supplier_invoices (company_id, supplier_number, invoice_number)
    WHERE status <> 'cancelled';
