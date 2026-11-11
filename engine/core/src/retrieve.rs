//! Finding the passages of a book that a question is about.

use rusqlite::{params_from_iter, Connection};
use serde::Serialize;

use crate::embeddings::{content_words, Embedder};
use crate::vectors::VectorStore;
use crate::Result;

const RRF_K: f64 = 60.0;
const CANDIDATES: usize = 30;
const CURRENT_SECTION_BOOST: f64 = 0.01;
const CONCEPT_BOOST: f64 = 0.005;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Retrieved {
    pub id: i64,
    pub text: String,
    pub chapter_id: i64,
    pub section_id: i64,
    pub chapter_title: String,
    pub section_title: String,
    pub start_page: i64,
    pub end_page: i64,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Concept {
    pub id: i64,
    pub name: String,
    pub normalized_name: String,
    pub definition: Option<String>,
}

fn to_concept(row: &rusqlite::Row<'_>) -> rusqlite::Result<Concept> {
    Ok(Concept { id: row.get(0)?, name: row.get(1)?, normalized_name: row.get(2)?, definition: row.get(3)? })
}

/// What a question is about.
#[derive(Debug, Default)]
pub struct Subject {
    pub terms: Vec<String>,
    /// The book's concepts that the question names, the longest names first.
    pub concepts: Vec<Concept>,
}

fn marks(count: usize) -> String {
    vec!["?"; count].join(",")
}

/// Works out what the question is about from its content words and the book's known concepts.
pub fn identify_subject(conn: &Connection, book_id: i64, question: &str) -> Result<Subject> {
    let lowered = question.to_lowercase();
    let plain: Vec<&str> = lowered.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).filter(|word| !word.is_empty()).collect();
    let spaced = format!(" {} ", plain.join(" "));
    let mut statement = conn.prepare("SELECT id, name, normalized_name, definition FROM concepts WHERE book_id = ?")?;
    let all: Vec<Concept> = statement.query_map([book_id], to_concept)?.collect::<rusqlite::Result<_>>()?;
    let mut concepts: Vec<Concept> = all.into_iter().filter(|concept| spaced.contains(&format!(" {} ", concept.normalized_name))).collect();
    concepts.sort_by_key(|concept| std::cmp::Reverse(concept.normalized_name.chars().count()));
    Ok(Subject { terms: content_words(question), concepts })
}

/// Passages holding any of the terms, best first.
fn lexical_search(conn: &Connection, book_id: i64, terms: &[String], limit: usize) -> Result<Vec<i64>> {
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let query = terms.iter().map(|term| format!("\"{term}\"")).collect::<Vec<_>>().join(" OR ");
    let mut statement = conn.prepare(
        "SELECT p.id FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
         WHERE passages_fts MATCH ? AND p.book_id = ? ORDER BY bm25(passages_fts) LIMIT ?",
    )?;
    let rows = statement.query_map(rusqlite::params![query, book_id, limit as i64], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load(conn: &Connection, scored: &[(i64, f64)]) -> Result<Vec<Retrieved>> {
    if scored.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(&format!(
        "SELECT p.id, p.text, p.chapter_id, p.section_id, c.title, s.title, p.start_page, p.end_page
         FROM passages p JOIN chapters c ON c.id = p.chapter_id JOIN sections s ON s.id = p.section_id
         WHERE p.id IN ({})",
        marks(scored.len())
    ))?;
    let rows = statement.query_map(params_from_iter(scored.iter().map(|(id, _)| id)), |row| {
        Ok(Retrieved {
            id: row.get(0)?, text: row.get(1)?, chapter_id: row.get(2)?, section_id: row.get(3)?, chapter_title: row.get(4)?,
            section_title: row.get(5)?, start_page: row.get(6)?, end_page: row.get(7)?, score: 0.0,
        })
    })?;
    let found: Vec<Retrieved> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(scored.iter().filter_map(|(id, score)| found.iter().find(|passage| passage.id == *id).map(|passage| Retrieved { score: *score, ..passage.clone() })).collect())
}

fn concept_passages(conn: &Connection, subject: &Subject) -> Result<Vec<i64>> {
    if subject.concepts.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(&format!("SELECT DISTINCT passage_id FROM passage_concepts WHERE concept_id IN ({})", marks(subject.concepts.len())))?;
    let rows = statement.query_map(params_from_iter(subject.concepts.iter().map(|concept| concept.id)), |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Sorts the best first, leaving equals in the order they were found.
fn best_first<T>(items: &mut [T], score: impl Fn(&T) -> f64) {
    items.sort_by(|a, b| score(b).total_cmp(&score(a)));
}

/// Fuses the keyword ranking and the vector ranking.
///
/// A book only answers a question it shares a content word with: when the keyword search finds
/// nothing, nothing is returned, however alike the vectors happen to be.
pub fn retrieve(
    conn: &Connection, book_id: i64, question: &str, subject: &Subject, vectors: &VectorStore, embedder: &Embedder,
    limit: usize, section_id: Option<i64>,
) -> Result<Vec<Retrieved>> {
    let lexical = lexical_search(conn, book_id, &subject.terms, CANDIDATES)?;
    if lexical.is_empty() {
        return Ok(Vec::new());
    }
    let vector: Vec<i64> = vectors.search(conn, book_id, embedder, question, CANDIDATES)?.into_iter().map(|(id, _)| id).collect();

    let mut fused: Vec<(i64, f64)> = Vec::new();
    for ranking in [&lexical, &vector] {
        for (rank, id) in ranking.iter().enumerate() {
            let share = 1.0 / (RRF_K + rank as f64 + 1.0);
            match fused.iter_mut().find(|(known, _)| known == id) {
                Some((_, score)) => *score += share,
                None => fused.push((*id, share)),
            }
        }
    }
    for id in concept_passages(conn, subject)? {
        if let Some((_, score)) = fused.iter_mut().find(|(known, _)| *known == id) {
            *score += CONCEPT_BOOST;
        }
    }
    best_first(&mut fused, |(_, score)| *score);

    let mut results = load(conn, &fused)?;
    if let Some(section_id) = section_id {
        // The learner is most likely asking about what is open in front of them.
        results.iter_mut().filter(|passage| passage.section_id == section_id).for_each(|passage| passage.score += CURRENT_SECTION_BOOST);
        best_first(&mut results, |passage| passage.score);
    }
    results.truncate(limit);
    Ok(results)
}

/// Concepts attached to the retrieved passages, those the question names first.
pub fn related_concepts(conn: &Connection, subject: &Subject, passages: &[&Retrieved], limit: usize) -> Result<Vec<Concept>> {
    let mut named = subject.concepts.clone();
    if !passages.is_empty() {
        let mut statement = conn.prepare(&format!(
            "SELECT c.id, c.name, c.normalized_name, c.definition FROM concepts c
             JOIN passage_concepts pc ON pc.concept_id = c.id
             WHERE pc.passage_id IN ({}) AND c.definition IS NOT NULL
             GROUP BY c.id ORDER BY COUNT(*) DESC, c.importance DESC LIMIT ?",
            marks(passages.len())
        ))?;
        let values = passages.iter().map(|passage| passage.id).chain([limit as i64]);
        for concept in statement.query_map(params_from_iter(values), to_concept)? {
            let concept = concept?;
            if named.iter().all(|known| known.id != concept.id) {
                named.push(concept);
            }
        }
    }
    named.truncate(limit);
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{knowledge_library, scratch_folder, section_id, KnowledgeLibrary};

    fn ask(conn: &Connection, book: i64, question: &str, section: Option<i64>, folder: &str) -> Vec<Retrieved> {
        let directory = scratch_folder(folder);
        let subject = identify_subject(conn, book, question).unwrap();
        let found = retrieve(conn, book, question, &subject, &VectorStore::new(directory.clone()), &Embedder::Hashing, 5, section).unwrap();
        let _ = std::fs::remove_dir_all(directory);
        found
    }

    #[test]
    fn the_relevant_passage_comes_first_with_its_pages() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let found = ask(&conn, knowledge, "What do mitochondria produce?", None, "retrieve-relevant");
        assert!(found[0].text.starts_with("Mitochondria produce"));
        assert_eq!((found[0].start_page, found[0].section_title.as_str(), found[0].chapter_title.as_str()), (2, "Energy in cells", "Chapter 1: Cells"));
        assert!(found.len() <= 5 && found.windows(2).all(|pair| pair[0].score >= pair[1].score));
    }

    #[test]
    fn a_question_sharing_no_word_with_the_book_finds_nothing() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        assert!(ask(&conn, knowledge, "Which volcano erupted?", None, "retrieve-nothing").is_empty());
        assert!(ask(&conn, knowledge, "Why is it so?", None, "retrieve-filler").is_empty());
    }

    #[test]
    fn only_the_book_asked_about_is_searched() {
        let KnowledgeLibrary { conn, knowledge, physics } = knowledge_library();
        let found = ask(&conn, physics, "What carries energy?", None, "retrieve-one-book");
        assert!(!found.is_empty());
        let belongs = |id: i64| conn.query_row("SELECT book_id FROM passages WHERE id = ?", [id], |row| row.get::<_, i64>(0)).unwrap();
        assert!(found.iter().all(|passage| belongs(passage.id) == physics && belongs(passage.id) != knowledge));
    }

    #[test]
    fn the_section_being_read_is_preferred() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let glossary = section_id(&conn, "Glossary");
        assert_eq!(ask(&conn, knowledge, "gravity", Some(glossary), "retrieve-glossary")[0].section_id, glossary);
        let forces = section_id(&conn, "Forces");
        assert_eq!(ask(&conn, knowledge, "gravity", Some(forces), "retrieve-forces")[0].section_id, forces);
    }

    #[test]
    fn the_subject_names_the_books_concepts_the_question_mentions() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let subject = identify_subject(&conn, knowledge, "How does GRAVITY differ from a force, please?").unwrap();
        assert_eq!(subject.terms, ["gravity", "differ", "force"]);
        assert_eq!(subject.concepts.iter().map(|concept| concept.name.as_str()).collect::<Vec<_>>(), ["gravity", "force"]);
        // A concept is named only as a whole word.
        assert!(identify_subject(&conn, knowledge, "Is it forceful?").unwrap().concepts.is_empty());
    }

    #[test]
    fn related_concepts_put_those_the_question_names_first() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let question = "What is osmosis?";
        let subject = identify_subject(&conn, knowledge, question).unwrap();
        let found = ask(&conn, knowledge, "gravity and force", None, "retrieve-related");
        let passages: Vec<&Retrieved> = found.iter().collect();
        let related = related_concepts(&conn, &subject, &passages, 6).unwrap();
        assert_eq!(related[0].name, "osmosis");
        let mut others: Vec<&str> = related[1..].iter().map(|concept| concept.name.as_str()).collect();
        others.sort_unstable();
        assert_eq!(others, ["force", "gravity"]);
        assert_eq!(related_concepts(&conn, &subject, &passages, 1).unwrap().len(), 1);
        assert_eq!(related_concepts(&conn, &subject, &[], 6).unwrap().len(), 1);
    }
}
