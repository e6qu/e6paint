//! SQLite persistence: rooms, tokens, latest snapshots.

use std::path::Path;

pub use rusqlite::Connection;

/// Opens (creating) the relay database and ensures the schema.
pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let conn = Connection::open(path).map_err(|err| err.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rooms (
            id       TEXT PRIMARY KEY,
            token    TEXT NOT NULL,
            snapshot BLOB,
            updated_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .map_err(|err| err.to_string())?;
    // Schema v2: read-only token. Existing databases get the column added; the relay
    // is pre-1.0, so there is no migration promise beyond this.
    if let Err(err) = conn.execute_batch("ALTER TABLE rooms ADD COLUMN read_token TEXT;") {
        log::warn!("schema upgrade (read_token) skipped: {err}");
    }
    Ok(conn)
}

/// A stored room identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomRow {
    pub id: String,
    pub token: String,
    pub read_token: Option<String>,
    pub snapshot: Option<Vec<u8>>,
}

pub fn insert_room(
    conn: &Connection,
    id: &str,
    token: &str,
    read_token: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO rooms (id, token, read_token, updated_at) VALUES (?1, ?2, ?3, unixepoch())",
        rusqlite::params![id, token, read_token],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn load_room(conn: &Connection, id: &str) -> Result<Option<RoomRow>, String> {
    let mut statement = conn
        .prepare("SELECT id, token, read_token, snapshot FROM rooms WHERE id = ?1")
        .map_err(|err| err.to_string())?;
    let mut rows = statement.query([id]).map_err(|err| err.to_string())?;
    match rows.next().map_err(|err| err.to_string())? {
        Some(row) => Ok(Some(RoomRow {
            id: row.get(0).map_err(|err| err.to_string())?,
            token: row.get(1).map_err(|err| err.to_string())?,
            read_token: row.get(2).map_err(|err| err.to_string())?,
            snapshot: row.get(3).map_err(|err| err.to_string())?,
        })),
        None => Ok(None),
    }
}

pub fn save_snapshot(conn: &Connection, id: &str, snapshot: &[u8]) -> Result<(), String> {
    conn.execute(
        "UPDATE rooms SET snapshot = ?1, updated_at = unixepoch() WHERE id = ?2",
        rusqlite::params![snapshot, id],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_lifecycle_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "e6relay-test-{}-{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let conn = open(&path)?;
        insert_room(&conn, "room-1", "tok-1", "ro-tok-1")?;

        let row = load_room(&conn, "room-1")?.ok_or("room must exist")?;
        assert_eq!(row.token, "tok-1");
        assert_eq!(row.read_token.as_deref(), Some("ro-tok-1"));
        assert!(row.snapshot.is_none(), "fresh room has no snapshot");
        assert!(load_room(&conn, "missing")?.is_none());

        save_snapshot(&conn, "room-1", &[1, 2, 3])?;
        assert_eq!(
            load_room(&conn, "room-1")?
                .ok_or("room must exist")?
                .snapshot,
            Some(vec![1, 2, 3])
        );
        std::fs::remove_file(&path)?;
        Ok(())
    }
}
