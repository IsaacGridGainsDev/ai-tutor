//! Visualise, served from the Rust engine: climate data and the figures drawn from a book.
//!
//! These read and write the same database as the Python backend, which still processes documents
//! and runs the model. The shell creates the database and brings it up to date before either runs.

use std::path::PathBuf;

use serde_json::Value;
use tutor_core::{climate, content};

/// The data folder, chosen exactly as the Python backend chooses it.
pub(crate) fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TUTOR_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("InteractiveTutor")
}

/// Runs one piece of work against the library, turning any failure into a message for the learner.
pub(crate) fn with_library<T>(work: impl FnOnce(&rusqlite::Connection) -> tutor_core::Result<T>) -> Result<T, String> {
    let conn = tutor_core::open(&data_dir().join("tutor.sqlite3")).map_err(|error| error.to_string())?;
    work(&conn).map_err(|error| error.to_string())
}

#[tauri::command(async)]
pub fn book_visuals(book_id: i64, chapter_id: Option<i64>, section_id: Option<i64>) -> Result<content::Visuals, String> {
    with_library(|conn| content::analyse(conn, book_id, content::Scope { chapter_id, section_id }))
}

#[tauri::command(async)]
pub fn diagram_delete(id: i64) -> Result<(), String> {
    with_library(|conn| content::delete_diagram(conn, id))
}

#[tauri::command(async)]
pub fn climate_list(book_id: i64) -> Result<Vec<climate::Dataset>, String> {
    with_library(|conn| climate::list(conn, book_id))
}

#[tauri::command(async)]
pub fn climate_candidates(book_id: i64) -> Result<Vec<climate::Candidate>, String> {
    with_library(|conn| climate::find_in_book(conn, book_id))
}

#[tauri::command(async)]
pub fn climate_save(book_id: i64, draft: Value, id: Option<i64>) -> Result<climate::Dataset, String> {
    let draft: climate::Draft = serde_json::from_value(draft).map_err(|_| "The climate data could not be read. Every value must be a number.".to_string())?;
    with_library(|conn| climate::save(conn, book_id, draft, id))
}

#[tauri::command(async)]
pub fn climate_delete(id: i64) -> Result<(), String> {
    with_library(|conn| climate::delete(conn, id))
}

#[tauri::command(async)]
pub fn climate_parse(text: String) -> climate::Tables {
    climate::parse_text(&text)
}
