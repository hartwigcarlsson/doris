-- Projections of the payroll-{company_id} streams. Rebuildable from events.
-- No foreign key to companies or vouchers: each crate rebuilds its own
-- tables independently (doris_ledger::rebuild_projections empties vouchers).

CREATE TABLE employees (
    company_id               TEXT    NOT NULL,
    employee_id              TEXT    NOT NULL,
    name                     TEXT    NOT NULL,
    personal_identity_number TEXT    NOT NULL,
    monthly_salary           INTEGER NOT NULL,
    salary_account           INTEGER NOT NULL,
    active                   INTEGER NOT NULL,
    PRIMARY KEY (company_id, employee_id),
    -- A personnummer once per company, also among inactive employees.
    UNIQUE (company_id, personal_identity_number)
);

CREATE TABLE payroll_runs (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    pay_date       TEXT    NOT NULL,
    text           TEXT    NOT NULL,
    finalized      INTEGER NOT NULL, -- 1 between Finalized and Reopened
    updated_at     TEXT    NOT NULL,
    updated_by     TEXT    NOT NULL,
    PRIMARY KEY (company_id, payroll_run_id)
);

-- The draft's lines while open; the locked lines (with account and fee)
-- once finalized.
CREATE TABLE payroll_run_lines (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER NOT NULL,
    salary_account INTEGER, -- NULL while open
    fee_rate       INTEGER,
    fee            INTEGER,
    net            INTEGER,
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);

-- Every booking ever made, in order (rowid); the latest one is in force
-- unless a voucher corrects it.
CREATE TABLE payroll_run_bookings (
    company_id        TEXT    NOT NULL,
    payroll_run_id    TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    voucher_number    INTEGER NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, voucher_number),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
