-- Projections of the ledger-* streams' opening balances and closings.
-- Rebuildable from events, like 0006.

-- The first fiscal year's ingående balanser. Later years' are derived from
-- these plus every earlier year's lines on accounts 1000-2999.
CREATE TABLE opening_balances (
    company_id TEXT    NOT NULL,
    account    INTEGER NOT NULL,
    debit      INTEGER NOT NULL,
    credit     INTEGER NOT NULL,
    PRIMARY KEY (company_id, account)
);

-- One row per closed fiscal year; reopening removes it.
CREATE TABLE closed_fiscal_years (
    company_id        TEXT NOT NULL,
    fiscal_year_start TEXT NOT NULL,
    closed_at         TEXT NOT NULL,
    closed_by         TEXT NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start)
);
