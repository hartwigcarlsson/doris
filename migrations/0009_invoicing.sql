-- Customer and supplier registers: projections of the customers-{company}
-- and suppliers-{company} streams. `details` is the event's details as JSON.
CREATE TABLE customers (
    company_id TEXT    NOT NULL REFERENCES companies(company_id),
    number     INTEGER NOT NULL,
    details    TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);

CREATE TABLE suppliers (
    company_id TEXT    NOT NULL REFERENCES companies(company_id),
    number     INTEGER NOT NULL,
    details    TEXT    NOT NULL,
    active     INTEGER NOT NULL,
    PRIMARY KEY (company_id, number)
);
