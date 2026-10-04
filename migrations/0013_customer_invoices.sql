-- Customer invoices: the projection of the customer-invoices-{company}
-- streams. `details` is the invoice as listed (JSON).
CREATE TABLE customer_invoices (
    company_id      TEXT    NOT NULL REFERENCES companies(company_id),
    number          INTEGER NOT NULL,
    customer_number INTEGER NOT NULL,
    invoice_number  TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    details         TEXT    NOT NULL,
    PRIMARY KEY (company_id, number)
);

-- An issued invoice number is never reused, not even after cancelling.
CREATE UNIQUE INDEX customer_invoices_unique_number
    ON customer_invoices (company_id, invoice_number);
