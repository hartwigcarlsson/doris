-- Projections of the accounts-* and ledger-* streams. Rebuildable from events.
-- No foreign key to companies: each crate rebuilds its own tables
-- independently.

CREATE TABLE accounts (
    company_id TEXT    NOT NULL,
    number     INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);

CREATE TABLE vouchers (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    date              TEXT    NOT NULL,
    text              TEXT    NOT NULL,
    corrects          INTEGER,
    corrected_by      INTEGER,
    recorded_at       TEXT    NOT NULL,
    recorded_by       TEXT    NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number)
);

CREATE TABLE voucher_lines (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    line_no           INTEGER NOT NULL,
    account           INTEGER NOT NULL,
    debit             INTEGER NOT NULL,
    credit            INTEGER NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number, line_no),
    FOREIGN KEY (company_id, fiscal_year_start, number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);

-- Voucher numbers run 1, 2, 3… per company and fiscal year (BFL 5 kap. 7 §).
-- The primary key refuses a duplicate; this refuses a gap. Both back up the
-- domain logic, which decides the number inside the write transaction.
CREATE TRIGGER vouchers_numbered_without_gaps BEFORE INSERT ON vouchers
WHEN NEW.number IS NOT (
    SELECT COALESCE(MAX(number), 0) + 1 FROM vouchers
    WHERE company_id = NEW.company_id AND fiscal_year_start = NEW.fiscal_year_start
)
BEGIN SELECT RAISE(ABORT, 'voucher numbers must run without gaps'); END;
