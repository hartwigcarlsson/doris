CREATE TABLE events (
    global_position INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id       TEXT    NOT NULL,
    stream_version  INTEGER NOT NULL,
    event_type      TEXT    NOT NULL,
    schema_version  INTEGER NOT NULL,
    payload         TEXT    NOT NULL CHECK (json_valid(payload)),
    metadata        TEXT    NOT NULL CHECK (json_valid(metadata)),
    recorded_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (stream_id, stream_version)
);
CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
