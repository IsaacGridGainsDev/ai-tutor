//! Opening the library database and bringing its schema up to date.
//!
//! The migration files stay under `backend/app/db/migrations` while the Python backend still
//! exists, because its tests build databases from them too. Both sides record what they applied in
//! the same `schema_migrations` table, so whichever runs second finds nothing to do. The desktop
//! shell migrates before it starts the backend, so in the app that is always Python.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::{Error, Result};

/// Every migration, oldest first: its number, its file name, and its SQL.
pub const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "0001_initial.sql", include_str!("../../../backend/app/db/migrations/0001_initial.sql")),
    (2, "0002_search.sql", include_str!("../../../backend/app/db/migrations/0002_search.sql")),
    (3, "0003_climate.sql", include_str!("../../../backend/app/db/migrations/0003_climate.sql")),
    (4, "0004_diagrams.sql", include_str!("../../../backend/app/db/migrations/0004_diagrams.sql")),
];

/// Opens the library database, creating its folder and file when they do not exist yet.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| Error::Invalid(format!("The data folder {} cannot be used: {error}", folder.display())))?;
    }
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // Readers and the one writer do not block each other, so the window can list books while a
    // document is being processed.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub fn current_version(conn: &Connection) -> Result<i64> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (datetime('now'))
         )",
    )?;
    let version: Option<i64> = conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row.get(0))?;
    Ok(version.unwrap_or(0))
}

/// Applies the migrations not yet applied, each one whole or not at all. Returns the version reached.
pub fn migrate(conn: &Connection) -> Result<i64> {
    let mut version = current_version(conn)?;
    for (number, name, sql) in MIGRATIONS {
        if *number <= version {
            continue;
        }
        let script = format!("BEGIN IMMEDIATE;\n{sql}\n;INSERT INTO schema_migrations (version, name) VALUES ({number}, '{name}');\nCOMMIT;");
        if let Err(error) = conn.execute_batch(&script) {
            let _ = conn.execute_batch("ROLLBACK");
            // Another program may have applied it between the check and the attempt.
            if current_version(conn)? >= *number {
                version = *number;
                continue;
            }
            return Err(Error::Invalid(format!("The library database could not be updated ({name}): {error}")));
        }
        version = *number;
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    #[test]
    fn migrating_creates_the_required_tables() {
        let conn = fresh();
        assert_eq!(migrate(&conn).unwrap(), MIGRATIONS.len() as i64);
        let mut statement = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
        let tables: Vec<String> = statement.query_map([], |row| row.get(0)).unwrap().map(|name| name.unwrap()).collect();
        for expected in [
            "books", "chapters", "sections", "passages", "concepts", "learning_sessions", "conversations", "questions",
            "answers", "assessments", "mastery_records", "misconceptions", "revision_schedules", "model_configurations",
            "recent_searches", "climate_datasets", "diagrams", "app_settings",
        ] {
            assert!(tables.iter().any(|table| table == expected), "{expected} is missing");
        }
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let conn = fresh();
        let first = migrate(&conn).unwrap();
        assert_eq!(migrate(&conn).unwrap(), first);
        assert_eq!(current_version(&conn).unwrap(), first);
        let recorded: i64 = conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0)).unwrap();
        assert_eq!(recorded, first);
    }

    #[test]
    fn a_database_made_by_the_python_backend_is_left_alone() {
        // Python records the same numbers and file names, so nothing is applied a second time.
        let conn = fresh();
        current_version(&conn).unwrap();
        for (number, name, sql) in MIGRATIONS {
            conn.execute_batch(sql).unwrap();
            conn.execute("INSERT INTO schema_migrations (version, name) VALUES (?, ?)", rusqlite::params![number, name]).unwrap();
        }
        assert_eq!(migrate(&conn).unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn removing_a_book_removes_what_belongs_to_it() {
        let conn = fresh();
        migrate(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO books (id, title, file_name, file_path, file_hash) VALUES (1, 't', 'f', 'p', 'h');
             INSERT INTO chapters (id, book_id, ordinal, title, start_page, end_page, detection_method) VALUES (1, 1, 1, 'c', 1, 2, 'test');
             DELETE FROM books WHERE id = 1;",
        ).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM chapters", [], |row| row.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn the_search_index_follows_the_passages() {
        let conn = fresh();
        migrate(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO books (id, title, file_name, file_path, file_hash) VALUES (1, 't', 'f', 'p', 'h');
             INSERT INTO chapters (id, book_id, ordinal, title, start_page, end_page, detection_method) VALUES (1, 1, 1, 'c', 1, 2, 'test');
             INSERT INTO sections (id, chapter_id, ordinal, title, start_page, end_page) VALUES (1, 1, 1, 's', 1, 2);
             INSERT INTO passages (id, book_id, chapter_id, section_id, ordinal, text, start_page, end_page)
                 VALUES (1, 1, 1, 1, 1, 'Mitochondria produce energy', 1, 1);",
        ).unwrap();
        let find = |conn: &Connection| -> Option<i64> {
            // "produces" finds "produce": words are reduced to their stems.
            conn.query_row("SELECT rowid FROM passages_fts WHERE passages_fts MATCH 'produces'", [], |row| row.get(0)).ok()
        };
        assert_eq!(find(&conn), Some(1));
        conn.execute("DELETE FROM passages WHERE id = 1", []).unwrap();
        assert_eq!(find(&conn), None);
    }

    #[test]
    fn opening_creates_the_folder_and_keeps_data_between_openings() {
        let folder = std::env::temp_dir().join(format!("tutor-core-test-{}", std::process::id()));
        let path = folder.join("nested").join("tutor.sqlite3");
        {
            let conn = open(&path).unwrap();
            migrate(&conn).unwrap();
            conn.execute("INSERT INTO app_settings (key, value) VALUES ('voice', '\"calm\"')", []).unwrap();
        }
        let conn = open(&path).unwrap();
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
        let kept: String = conn.query_row("SELECT value FROM app_settings WHERE key = 'voice'", [], |row| row.get(0)).unwrap();
        assert_eq!((mode.as_str(), kept.as_str()), ("wal", "\"calm\""));
        drop(conn);
        let _ = std::fs::remove_dir_all(folder);
    }
}
