//! A book's concepts as the window lists them, and adding to them with the local model.
//!
//! The first set of concepts is found without a model when a book is processed; that still
//! happens in the Python document pipeline.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::grounding::{field, quote_appears_in};
use crate::llm::{complete, Llm, Message};
use crate::search::normalize_name;
use crate::{prompts, require_book, Error, ModelProblem, Result};

const MAX_EXCERPT_CHARS: usize = 6000;
/// How much a concept the model proposed counts for, beside 1.0 for a glossary entry.
const MODEL_WEIGHT: f64 = 0.8;

#[derive(Debug, Serialize)]
pub struct ConceptView {
    pub id: i64,
    pub name: String,
    pub definition: Option<String>,
    pub importance: f64,
    /// The first few pages where it is discussed.
    pub pages: Vec<i64>,
    /// Names of the concepts most tied to it.
    pub related: Vec<String>,
}

/// A book's concepts, the most important first.
pub fn book_concepts(conn: &Connection, book_id: i64, limit: i64) -> Result<Vec<ConceptView>> {
    require_book(conn, book_id)?;
    let mut statement = conn.prepare("SELECT id, name, definition, importance FROM concepts WHERE book_id = ? ORDER BY importance DESC, name LIMIT ?")?;
    let rows = statement.query_map([book_id, limit.clamp(1, 1000)], |row| {
        Ok(ConceptView { id: row.get(0)?, name: row.get(1)?, definition: row.get(2)?, importance: row.get(3)?, pages: Vec::new(), related: Vec::new() })
    })?;
    let mut views: Vec<ConceptView> = rows.collect::<rusqlite::Result<_>>()?;
    let mut pages = conn.prepare(
        "SELECT DISTINCT p.start_page FROM passages p JOIN passage_concepts pc ON pc.passage_id = p.id
         WHERE pc.concept_id = ? ORDER BY p.start_page LIMIT 8",
    )?;
    let mut related = conn.prepare(
        "SELECT DISTINCT c.name FROM concept_relations r
         JOIN concepts c ON c.id = CASE WHEN r.source_id = ?1 THEN r.target_id ELSE r.source_id END
         WHERE r.source_id = ?1 OR r.target_id = ?1 ORDER BY c.importance DESC LIMIT 6",
    )?;
    for view in &mut views {
        view.pages = pages.query_map([view.id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
        view.related = related.query_map([view.id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    }
    Ok(views)
}

/// Passages from the start of a chapter or section, as many as fit in what the model can read at
/// once. Returns the text sent and the passages it was made from.
pub(crate) fn excerpt<T>(passages: Vec<T>, text: impl Fn(&T) -> &str, max_chars: usize) -> (String, Vec<T>) {
    let mut joined = String::new();
    let mut used = 0;
    let mut included = Vec::new();
    for passage in passages {
        let length = text(&passage).chars().count();
        if used > 0 && used + length > max_chars {
            break;
        }
        joined.push_str(text(&passage));
        joined.push_str("\n\n");
        used += length + 2;
        included.push(passage);
    }
    (joined, included)
}

/// Asks the model for further concepts of a chapter, keeping only those whose quoted evidence
/// really is in the chapter. Returns how many concepts were new.
pub fn enrich_chapter(conn: &Connection, llm: &dyn Llm, chapter_id: i64, model: &str, options: &Value) -> Result<usize> {
    let chapter: Option<(i64, String)> = conn.query_row("SELECT book_id, title FROM chapters WHERE id = ?", [chapter_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let Some((book_id, title)) = chapter else { return Err(Error::NotFound("Chapter")) };
    let mut statement = conn.prepare("SELECT id, text FROM passages WHERE chapter_id = ? ORDER BY id")?;
    let passages: Vec<(i64, String)> = statement.query_map([chapter_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    if passages.is_empty() {
        return Ok(0);
    }
    let (sent, included) = excerpt(passages, |(_, text)| text, MAX_EXCERPT_CHARS);

    let messages = [Message::system(prompts::CONCEPT_EXTRACTION_SYSTEM), Message::user(prompts::concept_extraction_user(&title, &sent))];
    let content = complete(llm, model, &messages, &prompts::concept_extraction_schema(), options)?;
    let unreadable = || Error::Model(ModelProblem::Failed, "The model returned concepts that could not be read. Please try again.".into());
    let reply: Value = serde_json::from_str(&content).map_err(|_| unreadable())?;
    let items = reply["concepts"].as_array().ok_or_else(unreadable)?;

    let mut added = 0;
    let transaction = conn.unchecked_transaction()?;
    for item in items {
        let (name, definition, quote) = (field(item, "name"), field(item, "definition"), field(item, "quote"));
        let key = normalize_name(&name);
        let Some((source, _)) = included.iter().find(|(_, text)| quote_appears_in(&quote, text)) else { continue };
        if key.chars().count() < 3 {
            continue;
        }
        let existing: Option<(i64, Option<String>)> = transaction
            .query_row("SELECT id, definition FROM concepts WHERE book_id = ? AND normalized_name = ?", rusqlite::params![book_id, key], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        let concept_id = match existing {
            Some((id, known)) => {
                // What the book's own glossary says is never replaced by the model's wording.
                if known.is_none_or(|text| text.is_empty()) && !definition.is_empty() {
                    transaction.execute("UPDATE concepts SET definition = ? WHERE id = ?", rusqlite::params![definition, id])?;
                }
                id
            }
            None => {
                transaction.execute(
                    "INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, ?, ?, ?, ?)",
                    rusqlite::params![book_id, name, key, Some(&definition).filter(|text| !text.is_empty()), MODEL_WEIGHT],
                )?;
                added += 1;
                transaction.last_insert_rowid()
            }
        };
        transaction.execute("INSERT OR IGNORE INTO passage_concepts (passage_id, concept_id) VALUES (?, ?)", [*source, concept_id])?;
    }
    transaction.commit()?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::llm::testing::FakeLlm;
    use crate::testing::{knowledge_library, KnowledgeLibrary};

    fn by_name(conn: &Connection, book: i64) -> std::collections::HashMap<String, ConceptView> {
        book_concepts(conn, book, 200).unwrap().into_iter().map(|concept| (concept.name.to_lowercase(), concept)).collect()
    }

    fn cells(conn: &Connection) -> i64 {
        conn.query_row("SELECT id FROM chapters WHERE title LIKE '%Cells%'", [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn concepts_are_listed_with_their_pages_and_relations() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        conn.execute("INSERT INTO concept_relations (source_id, target_id, relation) SELECT a.id, b.id, 'defined_using' FROM concepts a, concepts b WHERE a.name = 'force' AND b.name = 'gravity'", []).unwrap();
        let listed = book_concepts(&conn, knowledge, 200).unwrap();
        assert_eq!(listed.iter().map(|concept| concept.name.as_str()).collect::<Vec<_>>(), ["force", "gravity", "osmosis"]);
        let concepts = by_name(&conn, knowledge);
        assert!(concepts["gravity"].definition.as_deref().unwrap().starts_with("The attraction between masses"));
        assert_eq!(concepts["gravity"].pages, [4, 5]);
        // A relation is shown from both of its ends.
        assert_eq!((concepts["force"].related.as_slice(), concepts["gravity"].related.as_slice()), (&["gravity".to_string()][..], &["force".to_string()][..]));
        assert!(concepts["osmosis"].related.is_empty());
        assert_eq!(book_concepts(&conn, knowledge, 1).unwrap().len(), 1);
        assert!(matches!(book_concepts(&conn, 999, 200), Err(Error::NotFound("Book"))));
    }

    #[test]
    fn enrichment_keeps_only_concepts_with_real_evidence() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::replying(json!({ "concepts": [
            { "name": "chemical energy", "definition": "Energy a cell uses.", "quote": "most of the chemical energy that a cell needs" },
            { "name": "dark matter", "definition": "Made up.", "quote": "dark matter binds the galaxies together firmly" },
            { "name": "it", "definition": "Too short to be a concept.", "quote": "most of the chemical energy that a cell needs" },
        ] }));
        assert_eq!(enrich_chapter(&conn, &fake, cells(&conn), "local:1", &json!({})).unwrap(), 1);
        let concepts = by_name(&conn, knowledge);
        assert!(concepts.contains_key("chemical energy") && !concepts.contains_key("dark matter") && !concepts.contains_key("it"));
        assert_eq!((concepts["chemical energy"].importance, concepts["chemical energy"].pages.as_slice()), (0.8, &[2][..]));
        // The model saw the chapter's own text, and only that chapter's.
        let prompt = fake.prompt(0);
        assert!(prompt.starts_with("Chapter: Chapter 1: Cells\n\nExcerpt:\nMitochondria produce") && !prompt.contains("Gravity"));
    }

    #[test]
    fn a_known_concept_is_not_added_twice_and_keeps_its_definition() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        conn.execute("UPDATE concepts SET definition = NULL WHERE name = 'osmosis'", []).unwrap();
        conn.execute("INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, 'membrane', 'membrane', 'From the glossary.', 1.0)", [knowledge]).unwrap();
        let fake = FakeLlm::replying(json!({ "concepts": [
            { "name": "The Osmosis", "definition": "Water crossing a membrane.", "quote": "Movement of water across a membrane" },
            { "name": "membrane", "definition": "The model's own words.", "quote": "a thin barrier that controls which substances" },
        ] }));
        assert_eq!(enrich_chapter(&conn, &fake, cells(&conn), "local:1", &json!({})).unwrap(), 0);
        let concepts = by_name(&conn, knowledge);
        assert_eq!(concepts["osmosis"].definition.as_deref(), Some("Water crossing a membrane."));
        assert_eq!(concepts["membrane"].definition.as_deref(), Some("From the glossary."));
        assert_eq!(concepts["membrane"].pages, [3]);
    }

    #[test]
    fn enrichment_reports_what_went_wrong() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let chapter = cells(&conn);
        for reply in [json!("not json"), json!({ "concepts": 3 })] {
            let refused = enrich_chapter(&conn, &FakeLlm::replying(reply), chapter, "local:1", &json!({}));
            assert_eq!(refused.unwrap_err().to_string(), "The model returned concepts that could not be read. Please try again.");
        }
        let fake = FakeLlm::new();
        fake.go_down();
        assert!(matches!(enrich_chapter(&conn, &fake, chapter, "local:1", &json!({})), Err(Error::Model(ModelProblem::Unavailable, _))));
        assert!(matches!(enrich_chapter(&conn, &fake, 999, "local:1", &json!({})), Err(Error::NotFound("Chapter"))));
        assert_eq!(by_name(&conn, knowledge).len(), 3);
    }

    #[test]
    fn a_long_chapter_is_cut_at_a_passage_and_evidence_must_be_in_what_was_sent() {
        let conn = crate::testing::library();
        let long = "Filler sentence about nothing in particular. ".repeat(100);
        let book = crate::testing::chaptered_book(&conn, "Long", &[("Chapter 1", &[("1.1", &[long.as_str(), long.as_str(), "The hidden passage defines a late idea clearly."])])]);
        let fake = FakeLlm::replying(json!({ "concepts": [{ "name": "late idea", "definition": "Defined late.", "quote": "defines a late idea clearly" }] }));
        let chapter: i64 = conn.query_row("SELECT id FROM chapters WHERE book_id = ?", [book], |row| row.get(0)).unwrap();
        assert_eq!(enrich_chapter(&conn, &fake, chapter, "local:1", &json!({})).unwrap(), 0);
        assert!(!fake.prompt(0).contains("hidden passage"));
    }
}
