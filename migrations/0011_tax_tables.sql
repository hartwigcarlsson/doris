-- Skatteverket's monthly tax tables: reference data, not events. Fetched
-- once per year and replaceable; what a run used is locked in its event.
CREATE TABLE tax_tables (
    year        INTEGER NOT NULL,
    table_no    INTEGER NOT NULL,
    kind        TEXT    NOT NULL CHECK (kind IN ('amount', 'percent')),
    income_from INTEGER NOT NULL,
    income_to   INTEGER,          -- NULL: no upper limit
    col1 INTEGER NOT NULL,
    col2 INTEGER NOT NULL,
    col3 INTEGER NOT NULL,
    col4 INTEGER NOT NULL,
    col5 INTEGER NOT NULL,
    col6 INTEGER NOT NULL,
    PRIMARY KEY (year, table_no, kind, income_from)
);

-- An employee's tax setting (EmployeeTaxChanged): table and column, or a
-- percentage; all NULL without one.
ALTER TABLE employees ADD COLUMN tax_table   INTEGER;
ALTER TABLE employees ADD COLUMN tax_column  INTEGER;
ALTER TABLE employees ADD COLUMN tax_percent INTEGER;

-- A run line's tax may be blank (computed), and a locked line records its
-- basis (JSON TaxBasis). SQLite can't drop NOT NULL, so the projection table
-- is recreated; lines locked before this step are manual.
CREATE TABLE payroll_run_lines_new (
    company_id     TEXT    NOT NULL,
    payroll_run_id TEXT    NOT NULL,
    employee_id    TEXT    NOT NULL,
    gross          INTEGER NOT NULL,
    tax            INTEGER,          -- NULL: computed from the setting
    salary_account INTEGER,          -- NULL while open
    fee_rate       INTEGER,
    fee            INTEGER,
    net            INTEGER,
    tax_basis      TEXT,             -- NULL while open
    PRIMARY KEY (company_id, payroll_run_id, employee_id),
    FOREIGN KEY (company_id, payroll_run_id)
        REFERENCES payroll_runs (company_id, payroll_run_id)
);
INSERT INTO payroll_run_lines_new (company_id, payroll_run_id, employee_id, gross, tax,
    salary_account, fee_rate, fee, net, tax_basis)
SELECT company_id, payroll_run_id, employee_id, gross, tax, salary_account, fee_rate, fee, net,
       CASE WHEN salary_account IS NULL THEN NULL ELSE '{"kind":"manual"}' END
FROM payroll_run_lines;
DROP TABLE payroll_run_lines;
ALTER TABLE payroll_run_lines_new RENAME TO payroll_run_lines;
