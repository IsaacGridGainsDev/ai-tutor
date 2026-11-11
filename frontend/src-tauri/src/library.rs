//! The library, search and settings, served from the Rust engine.

use serde_json::Value;
use tutor_core::{library, search, settings};

use crate::visual::with_library;

#[tauri::command(async)]
pub fn books_list(q: Option<String>, collection_id: Option<i64>) -> Result<Vec<library::Book>, String> {
    with_library(|conn| library::list_books(conn, q.as_deref(), collection_id))
}

#[tauri::command(async)]
pub fn book_set_collections(id: i64, collection_ids: Vec<i64>) -> Result<library::Book, String> {
    with_library(|conn| library::set_book_collections(conn, id, &collection_ids))
}

#[tauri::command(async)]
pub fn book_structure(id: i64) -> Result<Vec<library::Chapter>, String> {
    with_library(|conn| library::structure(conn, id))
}

#[tauri::command(async)]
pub fn section_passages(id: i64, q: Option<String>) -> Result<Vec<library::Passage>, String> {
    with_library(|conn| library::section_passages(conn, id, q.as_deref()))
}

#[tauri::command(async)]
pub fn collections_list() -> Result<Vec<library::Collection>, String> {
    with_library(library::list_collections)
}

#[tauri::command(async)]
pub fn collection_create(name: String) -> Result<library::Collection, String> {
    with_library(|conn| library::create_collection(conn, &name))
}

#[tauri::command(async)]
pub fn collection_delete(id: i64) -> Result<(), String> {
    with_library(|conn| library::delete_collection(conn, id))
}

#[tauri::command(async)]
pub fn search_library(q: String, scope: search::Scope, live: bool, limit: Option<i64>) -> Result<search::Found, String> {
    with_library(|conn| search::search(conn, &q, &scope, live, limit.unwrap_or(50)))
}

#[tauri::command(async)]
pub fn search_recent() -> Result<Vec<search::RecentSearch>, String> {
    with_library(search::recent)
}

#[tauri::command(async)]
pub fn search_remember(query: String, scope: search::ScopeKind, book_id: Option<i64>, result_title: Option<String>) -> Result<(), String> {
    with_library(|conn| search::remember(conn, &query, scope, book_id, result_title.as_deref()))
}

#[tauri::command(async)]
pub fn search_clear_recent() -> Result<(), String> {
    with_library(search::clear_recent)
}

#[tauri::command(async)]
pub fn settings_get() -> Result<settings::AppSettings, String> {
    with_library(settings::load)
}

#[tauri::command(async)]
pub fn settings_put(settings: Value) -> Result<settings::AppSettings, String> {
    let wanted: settings::AppSettings = serde_json::from_value(settings).map_err(|_| "The settings could not be read.".to_string())?;
    with_library(|conn| settings::save(conn, wanted))
}
