-- Underlag (räkenskapsinformation, BFL 7 kap.). Primary data, like events:
-- not a projection, never updated or deleted. A file is stored once.
CREATE TABLE attachment_files (
    sha256 TEXT    PRIMARY KEY,
    size   INTEGER NOT NULL,
    data   BLOB    NOT NULL
);
CREATE TRIGGER attachment_files_no_update BEFORE UPDATE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;
CREATE TRIGGER attachment_files_no_delete BEFORE DELETE ON attachment_files
BEGIN SELECT RAISE(ABORT, 'attachment files are append-only'); END;

-- Projection of AttachmentAdded. Rebuildable from events. Files are read
-- only through here, so a hash alone never reads another company's file.
CREATE TABLE voucher_attachments (
    company_id        TEXT    NOT NULL,
    fiscal_year_start TEXT    NOT NULL,
    number            INTEGER NOT NULL,
    position          INTEGER NOT NULL,
    sha256            TEXT    NOT NULL REFERENCES attachment_files (sha256),
    file_name         TEXT    NOT NULL,
    content_type      TEXT    NOT NULL,
    size              INTEGER NOT NULL,
    added_at          TEXT    NOT NULL,
    added_by          TEXT    NOT NULL,
    PRIMARY KEY (company_id, fiscal_year_start, number, position),
    UNIQUE (company_id, fiscal_year_start, number, sha256),
    FOREIGN KEY (company_id, fiscal_year_start, number)
        REFERENCES vouchers (company_id, fiscal_year_start, number)
);
