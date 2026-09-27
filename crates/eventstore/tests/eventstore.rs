use doris_eventstore::{Error, Metadata, NewEvent, append, begin, load, open, read_all};
use serde_json::json;

fn event(n: i64) -> NewEvent {
    NewEvent {
        event_type: "Counted".into(),
        schema_version: 1,
        payload: json!({ "type": "Counted", "n": n }),
    }
}

fn actor() -> Metadata {
    Metadata {
        actor: Some("tester".into()),
    }
}

#[tokio::test]
async fn appended_events_are_loaded_in_stream_order() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut tx = begin(&pool).await.unwrap();
    append(&mut tx, "counter-1", 0, &[event(1), event(2)], &actor())
        .await
        .unwrap();
    append(&mut tx, "counter-1", 2, &[event(3)], &actor())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    let events = load(&mut conn, "counter-1").await.unwrap();
    let versions: Vec<i64> = events.iter().map(|e| e.stream_version).collect();
    let ns: Vec<i64> = events
        .iter()
        .map(|e| e.payload["n"].as_i64().unwrap())
        .collect();
    assert_eq!(versions, [1, 2, 3]);
    assert_eq!(ns, [1, 2, 3]);
    assert_eq!(events[0].metadata, actor());
    assert!(events[0].recorded_at.ends_with('Z'));
}

#[tokio::test]
async fn append_with_stale_expected_version_is_a_conflict() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "counter-1", 0, &[event(1)], &actor())
        .await
        .unwrap();

    let err = append(&mut conn, "counter-1", 0, &[event(2)], &actor())
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            Error::Conflict {
                expected: 0,
                actual: 1,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(load(&mut conn, "counter-1").await.unwrap().len(), 1);
}

#[tokio::test]
async fn events_can_never_be_updated_or_deleted() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "counter-1", 0, &[event(1)], &actor())
        .await
        .unwrap();

    let update = sqlx::query("UPDATE events SET payload = '{}'")
        .execute(&mut *conn)
        .await
        .unwrap_err();
    let delete = sqlx::query("DELETE FROM events")
        .execute(&mut *conn)
        .await
        .unwrap_err();

    assert!(update.to_string().contains("append-only"), "{update}");
    assert!(delete.to_string().contains("append-only"), "{delete}");
}

#[tokio::test]
async fn read_all_returns_events_across_streams_after_a_position() {
    let pool = open("sqlite::memory:").await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    append(&mut conn, "a", 0, &[event(1)], &actor())
        .await
        .unwrap();
    append(&mut conn, "b", 0, &[event(2)], &actor())
        .await
        .unwrap();
    append(&mut conn, "a", 1, &[event(3)], &actor())
        .await
        .unwrap();

    let all = read_all(&mut conn, 0).await.unwrap();
    let after_first = read_all(&mut conn, all[0].global_position).await.unwrap();

    let streams: Vec<&str> = all.iter().map(|e| e.stream_id.as_str()).collect();
    assert_eq!(streams, ["a", "b", "a"]);
    assert_eq!(after_first.len(), 2);
    assert_eq!(after_first[0].stream_id, "b");
}

#[test]
fn from_tagged_takes_event_type_from_serde_tag() {
    #[derive(serde::Serialize)]
    #[serde(tag = "type")]
    enum Thing {
        Happened { x: i32 },
    }

    let event = NewEvent::from_tagged(&Thing::Happened { x: 7 }, 1).unwrap();

    assert_eq!(event.event_type, "Happened");
    assert_eq!(event.payload, json!({ "type": "Happened", "x": 7 }));
}
