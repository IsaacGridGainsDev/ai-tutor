//! The Interactive Tutor's engines. Code arrives here from the Python backend one stage at a time
//! (see `docs/plans/rust-migration.md`); both read and write the same SQLite database meanwhile.

pub mod ai;
pub mod answer;
pub mod climate;
pub mod concepts;
pub mod content;
pub mod db;
pub mod diagram;
pub mod embeddings;
pub mod grounding;
pub mod library;
pub mod llm;
pub mod prompts;
pub mod retrieve;
pub mod search;
pub mod settings;
pub mod vectors;

use std::fmt;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug)]
pub enum Error {
    /// The thing asked for does not exist. The text names it for the learner.
    NotFound(&'static str),
    /// The request cannot be carried out as given. The text says why.
    Invalid(String),
    /// The local model could not do what was asked. The text is for the learner.
    Model(ModelProblem, String),
    Database(rusqlite::Error),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelProblem {
    /// The model runtime is not running.
    Unavailable,
    /// The model asked for is not on this computer.
    NotInstalled,
    /// The model would send text to another computer, which is never allowed.
    Cloud,
    /// The runtime or the model's reply went wrong.
    Failed,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(what) => write!(formatter, "{what} not found"),
            Self::Invalid(reason) | Self::Model(_, reason) => formatter.write_str(reason),
            Self::Database(error) => write!(formatter, "The library database could not be read: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub use db::open;

/// Where in a book something was found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Origin {
    pub passage_id: i64,
    pub section_id: i64,
    pub section_title: String,
    pub start_page: i64,
    pub end_page: i64,
}

pub(crate) fn require_book(conn: &Connection, book_id: i64) -> Result<()> {
    let found: Option<i64> = conn.query_row("SELECT 1 FROM books WHERE id = ?", [book_id], |row| row.get(0)).ok();
    found.map(|_| ()).ok_or(Error::NotFound("Book"))
}

/// One decimal place, rounding halves away from zero.
pub(crate) fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
pub(crate) mod testing {
    use rusqlite::Connection;

    /// A folder for one test's files, under the system's temporary folder. Not created yet.
    pub fn scratch_folder(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("tutor-core-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// An empty library with the real schema.
    pub fn library() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrate(&conn).unwrap();
        conn
    }

    /// Adds a finished book. Each chapter is a title and its sections; each section a title and
    /// its passages. Pages run on from 2, one per section. Returns the book id.
    pub fn chaptered_book(conn: &Connection, title: &str, chapters: &[(&str, &[(&str, &[&str])])]) -> i64 {
        conn.execute(
            "INSERT INTO books (title, file_name, file_path, file_hash, status, progress, page_count) VALUES (?, ?, 'p', ?, 'ready', 1, 9)",
            [title, &format!("{title}.pdf"), title],
        ).unwrap();
        let book = conn.last_insert_rowid();
        let mut page = 1;
        for (ordinal, (chapter_title, sections)) in chapters.iter().enumerate() {
            conn.execute(
                "INSERT INTO chapters (book_id, ordinal, title, start_page, end_page, detection_method) VALUES (?, ?, ?, ?, ?, 'test')",
                rusqlite::params![book, ordinal as i64, chapter_title, page + 1, page + sections.len() as i64],
            ).unwrap();
            let chapter = conn.last_insert_rowid();
            for (position, (section_title, passages)) in sections.iter().enumerate() {
                page += 1;
                conn.execute(
                    "INSERT INTO sections (chapter_id, ordinal, title, start_page, end_page) VALUES (?, ?, ?, ?, ?)",
                    rusqlite::params![chapter, position as i64, section_title, page, page],
                ).unwrap();
                let section = conn.last_insert_rowid();
                for (index, text) in passages.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO passages (book_id, chapter_id, section_id, ordinal, text, start_page, end_page) VALUES (?, ?, ?, ?, ?, ?, ?)",
                        rusqlite::params![book, chapter, section, index as i64, text, page, page],
                    ).unwrap();
                }
            }
        }
        book
    }

    /// Records a concept and ties it to the passages of its book that mention it.
    pub fn concept(conn: &Connection, book: i64, name: &str, definition: Option<&str>, importance: f64) {
        conn.execute(
            "INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![book, name, crate::search::normalize_name(name), definition, importance],
        ).unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO passage_concepts (passage_id, concept_id) SELECT id, ? FROM passages WHERE book_id = ? AND text LIKE '%' || ? || '%'",
            rusqlite::params![id, book, name],
        ).unwrap();
    }

    pub struct KnowledgeLibrary {
        pub conn: Connection,
        pub knowledge: i64,
        pub physics: i64,
    }

    /// Two finished books, as the Python tests had them after processing: one with known facts in
    /// known places, and one that shares the word "energy" with it.
    pub fn knowledge_library() -> KnowledgeLibrary {
        let conn = library();
        let knowledge = chaptered_book(&conn, "Knowledge Book", &[
            ("Chapter 1: Cells", &[
                ("Energy in cells", &[
                    "Mitochondria produce most of the chemical energy that a cell needs to function.",
                    "The energy molecule made by mitochondria is called adenosine triphosphate.",
                ]),
                ("Cell membranes", &[
                    "A cell membrane is a thin barrier that controls which substances enter the cell.",
                    "Movement of water across a membrane is known as osmosis.",
                ]),
            ]),
            ("Chapter 2: Motion", &[
                ("Forces", &[
                    "Gravity pulls every object toward the centre of the planet at a steady acceleration.",
                    "A push or a pull acting on a body is called a force.",
                    "Lifting a body against gravity stores energy in it.",
                ]),
                ("Glossary", &[
                    "gravity: The attraction between masses that pulls objects toward the planet.",
                    "force: A push or pull that changes the motion of a body, as gravity does.",
                ]),
            ]),
        ]);
        for (name, definition, importance) in [
            ("osmosis", Some("Movement of water across a membrane."), 2.0),
            ("gravity", Some("The attraction between masses that pulls objects toward the planet."), 3.0),
            ("force", Some("A push or pull that changes the motion of a body."), 3.0),
        ] {
            concept(&conn, knowledge, name, definition, importance);
        }
        let physics = chaptered_book(&conn, "Physics Primer", &[
            ("Chapter 1: Topic 1", &[
                ("1.1 Subject 1 1", &["Every system has a measurable energy that is conserved.", "Heat is energy in transit between bodies."]),
                ("1.2 Subject 1 2", &["Work transfers energy to a body by a force acting over a distance."]),
            ]),
            ("Chapter 2: Topic 2", &[("2.1 Subject 2 1", &["Waves carry energy without carrying matter.", "Light is a wave."])]),
        ]);
        concept(&conn, physics, "energy", None, 1.0);
        KnowledgeLibrary { conn, knowledge, physics }
    }

    /// Adds a book of one chapter whose sections each hold the given passages. Returns the book id.
    pub fn book(conn: &Connection, title: &str, sections: &[(&str, &[&str])]) -> i64 {
        conn.execute(
            "INSERT INTO books (title, file_name, file_path, file_hash, status) VALUES (?, 'f', 'p', ?, 'ready')",
            [title, title],
        ).unwrap();
        let book = conn.last_insert_rowid();
        conn.execute("INSERT INTO chapters (book_id, ordinal, title, start_page, end_page, detection_method) VALUES (?, 0, 'Chapter', 1, 9, 'test')", [book]).unwrap();
        let chapter = conn.last_insert_rowid();
        for (ordinal, (name, passages)) in sections.iter().enumerate() {
            let page = ordinal as i64 + 2;
            conn.execute(
                "INSERT INTO sections (chapter_id, ordinal, title, start_page, end_page) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![chapter, ordinal as i64, name, page, page],
            ).unwrap();
            let section = conn.last_insert_rowid();
            for (position, text) in passages.iter().enumerate() {
                conn.execute(
                    "INSERT INTO passages (book_id, chapter_id, section_id, ordinal, text, start_page, end_page) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![book, chapter, section, position as i64, text, page, page],
                ).unwrap();
            }
        }
        book
    }

    pub fn section_id(conn: &Connection, title: &str) -> i64 {
        conn.query_row("SELECT id FROM sections WHERE title = ?", [title], |row| row.get(0)).unwrap()
    }
}
