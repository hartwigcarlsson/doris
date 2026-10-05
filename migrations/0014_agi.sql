-- Projections of AgiContactChanged and AgiMonthSubmitted (payroll-* streams).
-- Rebuildable from events.
CREATE TABLE agi_contacts (
    company_id TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    phone      TEXT NOT NULL,
    email      TEXT NOT NULL
);

-- Every submission, in order; the latest per period is what Skatteverket has.
CREATE TABLE agi_submissions (
    company_id   TEXT    NOT NULL,
    period       INTEGER NOT NULL,
    submitted_at TEXT    NOT NULL,
    submitted_by TEXT    NOT NULL,
    lines        TEXT    NOT NULL, -- JSON Vec<AgiLine>
    fee_sum      INTEGER NOT NULL,
    tax_sum      INTEGER NOT NULL
);
