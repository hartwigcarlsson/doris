//! Append-only event log in SQLite: the source of truth for Doris.
//!
//! Writers must hold an IMMEDIATE transaction (see [`begin`]) so version
//! checks and projection updates are serialized.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("stream {stream} is at version {actual}, expected {expected}")]
    Conflict {
        stream: String,
        expected: i64,
        actual: i64,
    },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Who caused an event. Stored with every event (behandlingshistorik).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub actor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub event_type: String,
    pub schema_version: i64,
    pub payload: Value,
}

impl NewEvent {
    /// Builds an event from a serde enum tagged with `#[serde(tag = "type")]`.
    pub fn from_tagged<T: Serialize>(event: &T, schema_version: i64) -> Result<Self, Error> {
        let payload = serde_json::to_value(event)?;
        let event_type = payload["type"]
            .as_str()
            .expect("events must use #[serde(tag = \"type\")]")
            .to_owned();
        Ok(Self {
            event_type,
            schema_version,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedEvent {
    pub global_position: i64,
    pub stream_id: String,
    pub stream_version: i64,
    pub event_type: String,
    pub schema_version: i64,
    pub payload: Value,
    pub metadata: Metadata,
    pub recorded_at: String,
}

impl RecordedEvent {
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }

    fn from_row(row: &SqliteRow) -> Result<Self, Error> {
        Ok(Self {
            global_position: row.try_get("global_position")?,
            stream_id: row.try_get("stream_id")?,
            stream_version: row.try_get("stream_version")?,
            event_type: row.try_get("event_type")?,
            schema_version: row.try_get("schema_version")?,
            payload: serde_json::from_str(row.try_get("payload")?)?,
            metadata: serde_json::from_str(row.try_get("metadata")?)?,
            recorded_at: row.try_get("recorded_at")?,
        })
    }
}

/// Opens (creating if missing) the database and runs all migrations.
pub async fn open(url: &str) -> Result<SqlitePool, Error> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true);
    // Every connection to `:memory:` is its own database, so keep exactly one.
    let max_connections = if url.contains(":memory:") { 1 } else { 8 };
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

/// Starts a write transaction that takes the SQLite write lock up front.
pub async fn begin(pool: &SqlitePool) -> Result<Transaction<'static, Sqlite>, Error> {
    Ok(pool.begin_with("BEGIN IMMEDIATE").await?)
}

/// Current version of a stream; 0 when it has no events.
pub async fn stream_version(conn: &mut SqliteConnection, stream: &str) -> Result<i64, Error> {
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(stream_version) FROM events WHERE stream_id = ?")
            .bind(stream)
            .fetch_one(&mut *conn)
            .await?;
    Ok(version.unwrap_or(0))
}

/// Appends events to a stream, failing with [`Error::Conflict`] unless the
/// stream is at `expected_version`.
pub async fn append(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[NewEvent],
    metadata: &Metadata,
) -> Result<Vec<RecordedEvent>, Error> {
    let actual = stream_version(conn, stream).await?;
    if actual != expected_version {
        return Err(Error::Conflict {
            stream: stream.to_owned(),
            expected: expected_version,
            actual,
        });
    }
    let metadata = serde_json::to_string(metadata)?;
    let mut recorded = Vec::with_capacity(events.len());
    for (version, event) in (expected_version + 1..).zip(events) {
        let row = sqlx::query(
            "INSERT INTO events (stream_id, stream_version, event_type, schema_version, payload, metadata)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(stream)
        .bind(version)
        .bind(&event.event_type)
        .bind(event.schema_version)
        .bind(event.payload.to_string())
        .bind(&metadata)
        .fetch_one(&mut *conn)
        .await?;
        recorded.push(RecordedEvent::from_row(&row)?);
    }
    Ok(recorded)
}

pub async fn load(conn: &mut SqliteConnection, stream: &str) -> Result<Vec<RecordedEvent>, Error> {
    sqlx::query("SELECT * FROM events WHERE stream_id = ? ORDER BY stream_version")
        .bind(stream)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(RecordedEvent::from_row)
        .collect()
}

/// All events after `position`, in global order.
// ponytail: loads everything into memory; stream in batches when the log gets large.
pub async fn read_all(
    conn: &mut SqliteConnection,
    position: i64,
) -> Result<Vec<RecordedEvent>, Error> {
    sqlx::query("SELECT * FROM events WHERE global_position > ? ORDER BY global_position")
        .bind(position)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(RecordedEvent::from_row)
        .collect()
}
