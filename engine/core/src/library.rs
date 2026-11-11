//! The library as the window shows it: books, their collections, their structure and their text.
//!
//! Adding and removing a book stay with the Python backend until the document pipeline moves
//! (stage 4 of `docs/plans/rust-migration.md`): both involve the stored PDF and the work queue.

use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection, ErrorCode, OptionalExtension, Row};
use serde::Serialize;

use crate::{search, Error, Result};

const MAX_COLLECTION_NAME_CHARS: usize = 100;

#[derive(Debug, Serialize)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub file_name: String,
    pub page_count: i64,
    /// queued, extracting, ocr, structuring, indexing, ready or failed.
    pub status: String,
    pub status_detail: Option<String>,
    /// 0 to 1, how far processing has got.
    pub progress: f64,
    pub needs_ocr: bool,
    pub created_at: String,
    pub chapter_count: i64,
    pub passage_count: i64,
    pub collection_ids: Vec<i64>,
}

const BOOK_SELECT: &str = "
    SELECT b.id, b.title, b.author, b.file_name, b.page_count, b.status, b.status_detail, b.progress, b.needs_ocr, b.created_at,
        (SELECT COUNT(*) FROM chapters c WHERE c.book_id = b.id),
        (SELECT COUNT(*) FROM passages p WHERE p.book_id = b.id),
        (SELECT group_concat(bc.collection_id) FROM book_collections bc WHERE bc.book_id = b.id)
    FROM books b";

fn to_book(row: &Row<'_>) -> rusqlite::Result<Book> {
    let collections: Option<String> = row.get(12)?;
    let mut collection_ids: Vec<i64> = collections.iter().flat_map(|ids| ids.split(',')).filter_map(|id| id.parse().ok()).collect();
    collection_ids.sort_unstable();
    Ok(Book {
        id: row.get(0)?,
        title: row.get(1)?,
        author: row.get(2)?,
        file_name: row.get(3)?,
        page_count: row.get(4)?,
        status: row.get(5)?,
        status_detail: row.get(6)?,
        progress: row.get(7)?,
        needs_ocr: row.get(8)?,
        created_at: row.get(9)?,
        chapter_count: row.get(10)?,
        passage_count: row.get(11)?,
        collection_ids,
    })
}

fn like_pattern(text: &str) -> String {
    format!("%{}%", text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

/// The books, newest first, optionally only those whose title contains some text or that are in a collection.
pub fn list_books(conn: &Connection, title_contains: Option<&str>, collection_id: Option<i64>) -> Result<Vec<Book>> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(text) = title_contains.filter(|text| !text.is_empty()) {
        clauses.push("b.title LIKE ? ESCAPE '\\'");
        values.push(Sql::Text(like_pattern(text)));
    }
    if let Some(collection_id) = collection_id {
        clauses.push("EXISTS (SELECT 1 FROM book_collections x WHERE x.book_id = b.id AND x.collection_id = ?)");
        values.push(Sql::Integer(collection_id));
    }
    let filter = if clauses.is_empty() { String::new() } else { format!(" WHERE {}", clauses.join(" AND ")) };
    let mut statement = conn.prepare(&format!("{BOOK_SELECT}{filter} ORDER BY b.created_at DESC, b.id DESC"))?;
    let rows = statement.query_map(params_from_iter(values), to_book)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn book(conn: &Connection, book_id: i64) -> Result<Book> {
    conn.query_row(&format!("{BOOK_SELECT} WHERE b.id = ?"), [book_id], to_book).optional()?.ok_or(Error::NotFound("Book"))
}

fn broke_a_rule(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(failure, _) if failure.code == ErrorCode::ConstraintViolation)
}

/// Puts a book in exactly the given collections.
pub fn set_book_collections(conn: &Connection, book_id: i64, collection_ids: &[i64]) -> Result<Book> {
    book(conn, book_id)?;
    let transaction = conn.unchecked_transaction()?;
    transaction.execute("DELETE FROM book_collections WHERE book_id = ?", [book_id])?;
    for collection_id in collection_ids {
        let added = transaction.execute("INSERT OR IGNORE INTO book_collections (book_id, collection_id) VALUES (?, ?)", [book_id, *collection_id]);
        match added {
            Ok(_) => {}
            // Leaving the transaction unfinished undoes the removal above.
            Err(error) if broke_a_rule(&error) => return Err(Error::NotFound("Collection")),
            Err(error) => return Err(error.into()),
        }
    }
    transaction.commit()?;
    book(conn, book_id)
}

#[derive(Debug, Serialize)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub book_count: i64,
}

pub fn list_collections(conn: &Connection) -> Result<Vec<Collection>> {
    let mut statement = conn.prepare(
        "SELECT c.id, c.name, COUNT(bc.book_id)
         FROM collections c LEFT JOIN book_collections bc ON bc.collection_id = c.id
         GROUP BY c.id ORDER BY c.name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| Ok(Collection { id: row.get(0)?, name: row.get(1)?, book_count: row.get(2)? }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create_collection(conn: &Connection, name: &str) -> Result<Collection> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("Collection name cannot be blank.".into()));
    }
    if name.chars().count() > MAX_COLLECTION_NAME_CHARS {
        return Err(Error::Invalid(format!("A collection name can be at most {MAX_COLLECTION_NAME_CHARS} characters.")));
    }
    match conn.execute("INSERT INTO collections (name) VALUES (?)", [name]) {
        Ok(_) => Ok(Collection { id: conn.last_insert_rowid(), name: name.to_string(), book_count: 0 }),
        Err(error) if broke_a_rule(&error) => Err(Error::Invalid("A collection with that name already exists.".into())),
        Err(error) => Err(error.into()),
    }
}

/// Removes a collection. Its books stay in the library.
pub fn delete_collection(conn: &Connection, collection_id: i64) -> Result<()> {
    match conn.execute("DELETE FROM collections WHERE id = ?", [collection_id])? {
        0 => Err(Error::NotFound("Collection")),
        _ => Ok(()),
    }
}

#[derive(Debug, Serialize)]
pub struct Section {
    pub id: i64,
    pub title: String,
    pub start_page: i64,
    pub end_page: i64,
    pub passage_count: i64,
}

#[derive(Debug, Serialize)]
pub struct Chapter {
    pub id: i64,
    pub title: String,
    pub start_page: i64,
    pub end_page: i64,
    /// How the chapter was found: from the PDF's outline, from its headings, or as a fallback.
    pub detection_method: String,
    pub sections: Vec<Section>,
}

/// A book's chapters and their sections, in reading order.
pub fn structure(conn: &Connection, book_id: i64) -> Result<Vec<Chapter>> {
    book(conn, book_id)?;
    let mut statement = conn.prepare("SELECT id, title, start_page, end_page, detection_method FROM chapters WHERE book_id = ? ORDER BY ordinal")?;
    let chapters = statement.query_map([book_id], |row| {
        Ok(Chapter { id: row.get(0)?, title: row.get(1)?, start_page: row.get(2)?, end_page: row.get(3)?, detection_method: row.get(4)?, sections: Vec::new() })
    })?;
    let mut chapters: Vec<Chapter> = chapters.collect::<rusqlite::Result<_>>()?;

    let mut statement = conn.prepare(
        "SELECT s.chapter_id, s.id, s.title, s.start_page, s.end_page, (SELECT COUNT(*) FROM passages p WHERE p.section_id = s.id)
         FROM sections s JOIN chapters c ON c.id = s.chapter_id
         WHERE c.book_id = ? ORDER BY s.ordinal",
    )?;
    let sections = statement.query_map([book_id], |row| {
        Ok((row.get::<_, i64>(0)?, Section { id: row.get(1)?, title: row.get(2)?, start_page: row.get(3)?, end_page: row.get(4)?, passage_count: row.get(5)? }))
    })?;
    for section in sections {
        let (chapter_id, section) = section?;
        if let Some(chapter) = chapters.iter_mut().find(|chapter| chapter.id == chapter_id) {
            chapter.sections.push(section);
        }
    }
    Ok(chapters)
}

#[derive(Debug, Serialize)]
pub struct Passage {
    pub id: i64,
    pub section_id: i64,
    pub ordinal: i64,
    pub text: String,
    pub start_page: i64,
    pub end_page: i64,
    /// The text with search matches wrapped in the search marks, when a query was given and matched.
    pub highlighted: Option<String>,
}

/// A section's passages in reading order, with the words of a query marked when one is given.
pub fn section_passages(conn: &Connection, section_id: i64, query: Option<&str>) -> Result<Vec<Passage>> {
    let known: Option<i64> = conn.query_row("SELECT 1 FROM sections WHERE id = ?", [section_id], |row| row.get(0)).optional()?;
    if known.is_none() {
        return Err(Error::NotFound("Section"));
    }
    let mut statement = conn.prepare("SELECT id, section_id, ordinal, text, start_page, end_page FROM passages WHERE section_id = ? ORDER BY ordinal")?;
    let passages = statement.query_map([section_id], |row| {
        Ok(Passage { id: row.get(0)?, section_id: row.get(1)?, ordinal: row.get(2)?, text: row.get(3)?, start_page: row.get(4)?, end_page: row.get(5)?, highlighted: None })
    })?;
    let mut passages: Vec<Passage> = passages.collect::<rusqlite::Result<_>>()?;
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        for (passage_id, marked) in search::highlight_section(conn, section_id, query)? {
            if let Some(passage) = passages.iter_mut().find(|passage| passage.id == passage_id) {
                passage.highlighted = Some(marked);
            }
        }
    }
    Ok(passages)
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub page_number: i64,
    /// The number printed on the page, when the PDF records one.
    pub label: Option<String>,
    pub text: String,
    /// text, ocr or none: where the words came from.
    pub source: String,
}

/// One page of a book as it was read, before it was divided into passages.
pub fn page(conn: &Connection, book_id: i64, page_number: i64) -> Result<Page> {
    conn.query_row(
        "SELECT page_number, label, text, source FROM pages WHERE book_id = ? AND page_number = ?",
        [book_id, page_number],
        |row| Ok(Page { page_number: row.get(0)?, label: row.get(1)?, text: row.get(2)?, source: row.get(3)? }),
    ).optional()?.ok_or(Error::NotFound("Page"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{MARK_END, MARK_START};
    use crate::testing::{chaptered_book, knowledge_library, library, section_id, KnowledgeLibrary};

    fn ids(books: &[Book]) -> Vec<i64> {
        books.iter().map(|book| book.id).collect()
    }

    #[test]
    fn books_are_listed_newest_first_with_their_counts() {
        let KnowledgeLibrary { conn, knowledge, physics } = knowledge_library();
        let books = list_books(&conn, None, None).unwrap();
        assert_eq!(ids(&books), [physics, knowledge]);
        let first = book(&conn, knowledge).unwrap();
        assert_eq!((first.title.as_str(), first.status.as_str(), first.progress, first.needs_ocr), ("Knowledge Book", "ready", 1.0, false));
        assert_eq!((first.chapter_count, first.passage_count, first.file_name.as_str()), (2, 9, "Knowledge Book.pdf"));
        assert!(first.collection_ids.is_empty() && first.author.is_none());
        assert!(matches!(book(&conn, 999), Err(Error::NotFound("Book"))));
    }

    #[test]
    fn a_book_being_processed_reports_how_far_it_has_got() {
        let conn = library();
        conn.execute("INSERT INTO books (title, file_name, file_path, file_hash, status, status_detail, progress, needs_ocr) VALUES ('Scan', 's.pdf', 'p', 'h', 'ocr', 'Reading page 3 of 9', 0.3, 1)", []).unwrap();
        let listed = &list_books(&conn, None, None).unwrap()[0];
        assert_eq!((listed.status.as_str(), listed.status_detail.as_deref(), listed.progress, listed.needs_ocr), ("ocr", Some("Reading page 3 of 9"), 0.3, true));
        assert_eq!((listed.chapter_count, listed.passage_count), (0, 0));
    }

    #[test]
    fn books_are_filtered_by_title_and_collection() {
        let conn = library();
        let chemistry = chaptered_book(&conn, "Organic Chemistry", &[]);
        let algebra = chaptered_book(&conn, "Linear Algebra", &[]);
        let science = create_collection(&conn, "Science").unwrap();
        assert_eq!(create_collection(&conn, "Science").unwrap_err().to_string(), "A collection with that name already exists.");
        let filed = set_book_collections(&conn, chemistry, &[science.id, science.id]).unwrap();
        assert_eq!(filed.collection_ids, [science.id]);

        assert_eq!(ids(&list_books(&conn, None, Some(science.id)).unwrap()), [chemistry]);
        assert_eq!(ids(&list_books(&conn, Some("algebra"), None).unwrap()), [algebra]);
        assert_eq!(ids(&list_books(&conn, Some("a"), Some(science.id)).unwrap()), [chemistry]);
        // Signs with a meaning in the comparison are taken as written.
        assert!(list_books(&conn, Some("%"), None).unwrap().is_empty());
        assert!(list_books(&conn, Some("_"), None).unwrap().is_empty());
        assert_eq!(list_books(&conn, Some(""), None).unwrap().len(), 2);

        assert_eq!(list_collections(&conn).unwrap()[0].book_count, 1);
        delete_collection(&conn, science.id).unwrap();
        assert!(book(&conn, chemistry).unwrap().collection_ids.is_empty());
        assert!(matches!(delete_collection(&conn, science.id), Err(Error::NotFound("Collection"))));
    }

    #[test]
    fn collections_are_listed_by_name_and_need_a_name() {
        let conn = library();
        for name in ["zoology", "  Botany  ", "anatomy"] {
            create_collection(&conn, name).unwrap();
        }
        let names: Vec<String> = list_collections(&conn).unwrap().into_iter().map(|collection| collection.name).collect();
        assert_eq!(names, ["anatomy", "Botany", "zoology"]);
        assert_eq!(create_collection(&conn, "   ").unwrap_err().to_string(), "Collection name cannot be blank.");
        assert!(create_collection(&conn, &"n".repeat(101)).is_err());
        assert!(create_collection(&conn, &"é".repeat(100)).is_ok());
    }

    #[test]
    fn filing_under_an_unknown_collection_changes_nothing() {
        let conn = library();
        let book_id = chaptered_book(&conn, "Book", &[]);
        let kept = create_collection(&conn, "Kept").unwrap();
        set_book_collections(&conn, book_id, &[kept.id]).unwrap();
        assert!(matches!(set_book_collections(&conn, book_id, &[kept.id, 999]), Err(Error::NotFound("Collection"))));
        assert_eq!(book(&conn, book_id).unwrap().collection_ids, [kept.id]);
        assert!(matches!(set_book_collections(&conn, 999, &[kept.id]), Err(Error::NotFound("Book"))));
        assert!(set_book_collections(&conn, book_id, &[]).unwrap().collection_ids.is_empty());
    }

    #[test]
    fn structure_lists_chapters_and_sections_in_reading_order() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let chapters = structure(&conn, knowledge).unwrap();
        assert_eq!(chapters.iter().map(|chapter| chapter.title.as_str()).collect::<Vec<_>>(), ["Chapter 1: Cells", "Chapter 2: Motion"]);
        assert_eq!(chapters[1].sections.iter().map(|section| section.title.as_str()).collect::<Vec<_>>(), ["Forces", "Glossary"]);
        assert_eq!((chapters[0].start_page, chapters[0].end_page, chapters[0].detection_method.as_str()), (2, 3, "test"));
        assert_eq!((chapters[1].sections[0].passage_count, chapters[1].sections[0].start_page), (3, 4));
        assert!(matches!(structure(&conn, 999), Err(Error::NotFound("Book"))));
    }

    #[test]
    fn passages_come_in_order_and_unmarked_without_a_query() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let section = section_id(&conn, "Energy in cells");
        for query in [None, Some("")] {
            let passages = section_passages(&conn, section, query).unwrap();
            assert_eq!(passages.iter().map(|passage| passage.ordinal).collect::<Vec<_>>(), [0, 1]);
            assert!(passages[0].text.starts_with("Mitochondria produce") && passages.iter().all(|passage| passage.highlighted.is_none()));
            assert_eq!((passages[0].section_id, passages[0].start_page, passages[0].end_page), (section, 2, 2));
        }
        assert!(matches!(section_passages(&conn, 999, None), Err(Error::NotFound("Section"))));
    }

    #[test]
    fn a_query_marks_only_the_passages_it_matches() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let passages = section_passages(&conn, section_id(&conn, "Cell membranes"), Some("osmosis")).unwrap();
        assert!(passages[0].highlighted.is_none());
        let marked = passages[1].highlighted.as_deref().unwrap();
        assert!(marked.ends_with(&format!("{MARK_START}osmosis{MARK_END}.")));
        assert_eq!(marked.replace([MARK_START, MARK_END], ""), passages[1].text);
    }

    #[test]
    fn a_page_is_read_as_it_was_extracted() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        conn.execute("INSERT INTO pages (book_id, page_number, label, text, source) VALUES (?, 3, 'iii', 'Words on the page', 'ocr')", [knowledge]).unwrap();
        let read = page(&conn, knowledge, 3).unwrap();
        assert_eq!((read.page_number, read.label.as_deref(), read.text.as_str(), read.source.as_str()), (3, Some("iii"), "Words on the page", "ocr"));
        assert!(matches!(page(&conn, knowledge, 4), Err(Error::NotFound("Page"))));
    }
}
