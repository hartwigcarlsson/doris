-- Projections of the company-* streams. Rebuildable from events.
-- user_id has no foreign key: users live in another crate's projection,
-- and each crate rebuilds its own tables independently.

CREATE TABLE companies (
    company_id              TEXT PRIMARY KEY,
    org_nr                  TEXT NOT NULL UNIQUE,
    name                    TEXT NOT NULL,
    legal_form              TEXT NOT NULL,
    street                  TEXT,
    postal_code             TEXT,
    city                    TEXT,
    first_fiscal_year_start TEXT NOT NULL,
    first_fiscal_year_end   TEXT NOT NULL,
    accounting_method       TEXT NOT NULL,
    registered_at           TEXT NOT NULL
);

CREATE TABLE company_members (
    company_id TEXT NOT NULL REFERENCES companies (company_id),
    user_id    TEXT NOT NULL,
    added_at   TEXT NOT NULL,
    PRIMARY KEY (company_id, user_id)
);

CREATE INDEX company_members_user ON company_members (user_id);
