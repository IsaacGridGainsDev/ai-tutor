//! Library search: scoped, phrase-aware full-text search with grouped, located results.

use std::sync::OnceLock;

use regex::Regex;
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};

use crate::{require_book, Error, Result};

/// Control characters cannot occur in extracted text, so the marks can never collide with a book's content.
pub const MARK_START: char = '\u{2}';
pub const MARK_END: char = '\u{3}';
const SNIPPET_TOKENS: usize = 24;
const PREFIX_MIN_CHARS: usize = 2;
const CONCEPT_LIMIT: i64 = 8;
const MAX_QUERY_CHARS: usize = 500;
const KEPT_RECENT: i64 = 20;

fn words(text: &str) -> Vec<String> {
    static WORD: OnceLock<Regex> = OnceLock::new();
    WORD.get_or_init(|| Regex::new(r"\w+").unwrap()).find_iter(text).map(|word| word.as_str().to_string()).collect()
}

/// Each part is a list of words: one word, or several for an exact phrase.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedQuery {
    pub parts: Vec<Vec<String>>,
    pub prefix_last: bool,
}

impl ParsedQuery {
    /// Quotes every part, so what the learner typed can never be read as search syntax.
    pub fn to_fts(&self, joiner: &str) -> String {
        let rendered: Vec<String> = self.parts.iter().enumerate().map(|(index, words)| {
            let star = if self.prefix_last && index == self.parts.len() - 1 { "*" } else { "" };
            format!("\"{}\"{star}", words.join(" "))
        }).collect();
        rendered.join(&format!(" {joiner} "))
    }
}

/// Splits a query into words and "exact phrases".
///
/// While typing (live), the final word is treated as a prefix, so "recur" already finds "recursion".
pub fn parse_query(text: &str, live: bool) -> ParsedQuery {
    static PART: OnceLock<Regex> = OnceLock::new();
    let part = PART.get_or_init(|| Regex::new(r#""([^"]*)"|(\S+)"#).unwrap());
    let mut parts = Vec::new();
    let mut last_is_word = false;
    for found in part.captures_iter(text) {
        if let Some(phrase) = found.get(1) {
            let phrase = words(phrase.as_str());
            if !phrase.is_empty() {
                parts.push(phrase);
                last_is_word = false;
            }
        } else {
            for word in words(&found[2]) {
                parts.push(vec![word]);
                last_is_word = true;
            }
        }
    }
    let still_typing = !text.chars().last().is_some_and(char::is_whitespace);
    let prefix_last = live && last_is_word && still_typing
        && parts.last().is_some_and(|last| last[0].chars().count() >= PREFIX_MIN_CHARS);
    ParsedQuery { parts, prefix_last }
}

/// A concept's name reduced to what is compared: lower case, plain words, no leading article.
pub fn normalize_name(name: &str) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let lowered = name.to_lowercase();
    let mut found: Vec<&str> = WORD.get_or_init(|| Regex::new("[a-z0-9]+").unwrap()).find_iter(&lowered).map(|word| word.as_str()).collect();
    if matches!(found.first(), Some(&("a" | "an" | "the"))) {
        found.remove(0);
    }
    found.join(" ")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    #[default]
    All,
    Book,
    Chapter,
    Section,
}

impl ScopeKind {
    fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Book => "book",
            Self::Chapter => "chapter",
            Self::Section => "section",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Scope {
    pub kind: ScopeKind,
    pub book_id: Option<i64>,
    pub chapter_id: Option<i64>,
    pub section_id: Option<i64>,
}

impl Scope {
    pub fn section(section_id: i64) -> Self {
        Self { kind: ScopeKind::Section, section_id: Some(section_id), ..Self::default() }
    }

    pub fn book(book_id: i64) -> Self {
        Self { kind: ScopeKind::Book, book_id: Some(book_id), ..Self::default() }
    }

    /// The passage column the scope filters on and the value it must have.
    fn target(&self) -> Result<Option<(&'static str, i64)>> {
        let (column, value) = match self.kind {
            ScopeKind::All => return Ok(None),
            ScopeKind::Book => ("book_id", self.book_id),
            ScopeKind::Chapter => ("chapter_id", self.chapter_id),
            ScopeKind::Section => ("section_id", self.section_id),
        };
        let name = self.kind.name();
        value.map(|value| Some((column, value))).ok_or_else(|| Error::Invalid(format!("A {name} scope needs a {name} id.")))
    }

    /// The filter for joins that reach passages some other way than the full-text index.
    fn clause(&self) -> Result<(String, Vec<i64>)> {
        Ok(match self.target()? {
            None => (String::new(), Vec::new()),
            Some((column, value)) => (format!(" AND p.{column} = ?"), vec![value]),
        })
    }

    /// The filter for full-text queries, or None when the scope holds no passages.
    ///
    /// A book's passages are stored together, so bounding the index scan by their id range lets a
    /// scoped search skip the rest of the library. The equality test keeps the result exact.
    fn fts_clause(&self, conn: &Connection) -> Result<Option<(String, Vec<i64>)>> {
        let Some((column, value)) = self.target()? else {
            return Ok(Some((String::new(), Vec::new())));
        };
        let (low, high): (Option<i64>, Option<i64>) = conn.query_row(
            &format!("SELECT MIN(id), MAX(id) FROM passages WHERE {column} = ?"),
            [value],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(match (low, high) {
            (Some(low), Some(high)) => Some((format!(" AND passages_fts.rowid BETWEEN ? AND ? AND p.{column} = ?"), vec![low, high, value])),
            _ => None,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Hit {
    pub passage_id: i64,
    pub book_id: i64,
    pub chapter_id: i64,
    pub chapter_title: String,
    pub section_id: i64,
    pub section_title: String,
    pub start_page: i64,
    pub end_page: i64,
    /// A few words either side of the match, which is wrapped in the marks.
    pub snippet: String,
    /// The text was read from a scanned page, so it may contain reading errors.
    pub from_ocr: bool,
}

#[derive(Debug, Serialize)]
pub struct BookGroup {
    pub book_id: i64,
    pub book_title: String,
    /// Every match in the book, including those beyond the hits shown.
    pub total: i64,
    pub hits: Vec<Hit>,
}

#[derive(Debug, Serialize)]
pub struct ConceptMatch {
    pub id: i64,
    pub name: String,
    pub definition: Option<String>,
    pub book_id: i64,
    pub book_title: String,
    pub passage_id: i64,
    pub section_id: i64,
    pub start_page: i64,
}

#[derive(Debug, Serialize)]
pub struct Found {
    pub query: String,
    /// "all" when every term matched, "any" when only some did, None when nothing was searched.
    pub match_mode: Option<&'static str>,
    pub total: i64,
    pub groups: Vec<BookGroup>,
    pub concepts: Vec<ConceptMatch>,
}

fn bound(expression: &str, scope: &[i64]) -> Vec<Sql> {
    std::iter::once(Sql::Text(expression.to_string())).chain(scope.iter().map(|&value| Sql::Integer(value))).collect()
}

/// The hits for one full-text expression, best first, each with the title of its book.
fn hits(conn: &Connection, expression: &str, clause: &str, scope: &[i64], limit: i64) -> Result<Vec<(String, Hit)>> {
    // CROSS JOIN fixes the join order: the full-text index first, then lookups by primary key. Left
    // to itself the planner can start from passages and probe the index once per passage, which is
    // far slower.
    let sql = format!(
        "SELECT p.id, p.book_id, b.title, p.chapter_id, c.title, p.section_id, s.title, p.start_page, p.end_page,
                snippet(passages_fts, 0, '{MARK_START}', '{MARK_END}', '…', {SNIPPET_TOKENS}),
                EXISTS (SELECT 1 FROM pages pg WHERE pg.book_id = p.book_id AND pg.source = 'ocr'
                        AND pg.page_number BETWEEN p.start_page AND p.end_page)
         FROM passages_fts
         CROSS JOIN passages p ON p.id = passages_fts.rowid
         CROSS JOIN books b ON b.id = p.book_id
         CROSS JOIN chapters c ON c.id = p.chapter_id
         CROSS JOIN sections s ON s.id = p.section_id
         WHERE passages_fts MATCH ?{clause} AND b.status = 'ready'
         ORDER BY passages_fts.rank
         LIMIT ?"
    );
    let mut values = bound(expression, scope);
    values.push(Sql::Integer(limit));
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        Ok((row.get::<_, String>(2)?, Hit {
            passage_id: row.get(0)?,
            book_id: row.get(1)?,
            chapter_id: row.get(3)?,
            chapter_title: row.get(4)?,
            section_id: row.get(5)?,
            section_title: row.get(6)?,
            start_page: row.get(7)?,
            end_page: row.get(8)?,
            snippet: row.get(9)?,
            from_ocr: row.get(10)?,
        }))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn counts(conn: &Connection, expression: &str, clause: &str, scope: &[i64]) -> Result<Vec<(i64, i64)>> {
    let sql = format!(
        "SELECT p.book_id, COUNT(*)
         FROM passages_fts CROSS JOIN passages p ON p.id = passages_fts.rowid CROSS JOIN books b ON b.id = p.book_id
         WHERE passages_fts MATCH ?{clause} AND b.status = 'ready'
         GROUP BY p.book_id"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(bound(expression, scope)), |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Concepts whose names match, found separately from the text, as OneNote finds tags.
fn concepts(conn: &Connection, text: &str, scope: &Scope) -> Result<Vec<ConceptMatch>> {
    let needle = normalize_name(text);
    if needle.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let escaped = needle.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let (clause, scope_values) = scope.clause()?;
    let sql = format!(
        "SELECT c.id, c.name, c.definition, c.book_id, b.title, MIN(p.id), MIN(p.start_page), p.section_id
         FROM concepts c
         JOIN books b ON b.id = c.book_id
         JOIN passage_concepts pc ON pc.concept_id = c.id
         JOIN passages p ON p.id = pc.passage_id
         WHERE c.normalized_name LIKE ? ESCAPE '\\' AND b.status = 'ready'{clause}
         GROUP BY c.id
         ORDER BY (c.normalized_name = ?) DESC, c.importance DESC
         LIMIT ?"
    );
    let mut values = vec![Sql::Text(format!("%{escaped}%"))];
    values.extend(scope_values.into_iter().map(Sql::Integer));
    values.push(Sql::Text(needle));
    values.push(Sql::Integer(CONCEPT_LIMIT));
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        Ok(ConceptMatch {
            id: row.get(0)?,
            name: row.get(1)?,
            definition: row.get(2)?,
            book_id: row.get(3)?,
            book_title: row.get(4)?,
            passage_id: row.get(5)?,
            start_page: row.get(6)?,
            section_id: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn too_long(text: &str) -> bool {
    text.chars().count() > MAX_QUERY_CHARS
}

/// Hits grouped by book, the best book first, with an honest total for each book.
pub fn search(conn: &Connection, text: &str, scope: &Scope, live: bool, limit: i64) -> Result<Found> {
    if too_long(text) {
        return Err(Error::Invalid(format!("A search can be at most {MAX_QUERY_CHARS} characters.")));
    }
    let mut found = Found { query: text.to_string(), match_mode: None, total: 0, groups: Vec::new(), concepts: Vec::new() };
    // The scope is checked even when there is nothing to search for, so a mistake in it is reported.
    let bounded = scope.fts_clause(conn)?;
    let parsed = parse_query(text, live);
    let (Some((clause, scope_values)), false) = (bounded, parsed.parts.is_empty()) else {
        return Ok(found);
    };
    let limit = limit.clamp(1, 200);

    // Every term first; any term only when that finds nothing and there is more than one term.
    let mut expression = String::new();
    let mut rows = Vec::new();
    for (mode, joiner) in [("all", "AND"), ("any", "OR")] {
        expression = parsed.to_fts(joiner);
        rows = hits(conn, &expression, &clause, &scope_values, limit)?;
        found.match_mode = Some(mode);
        if !rows.is_empty() || parsed.parts.len() == 1 {
            break;
        }
    }

    let totals = counts(conn, &expression, &clause, &scope_values)?;
    for (book_title, hit) in rows {
        let position = match found.groups.iter().position(|group| group.book_id == hit.book_id) {
            Some(position) => position,
            None => {
                let total = totals.iter().find(|(book, _)| *book == hit.book_id).map_or(0, |(_, total)| *total);
                found.groups.push(BookGroup { book_id: hit.book_id, book_title, total, hits: Vec::new() });
                found.groups.len() - 1
            }
        };
        found.groups[position].hits.push(hit);
    }
    found.total = totals.iter().map(|(_, total)| total).sum();
    found.concepts = concepts(conn, text, scope)?;
    Ok(found)
}

/// Each matching passage in a section with every query word marked, for stepping through the hits.
///
/// The last word matches as a prefix, as it does in live search, so whatever found a passage is
/// also what gets marked in it.
pub fn highlight_section(conn: &Connection, section_id: i64, text: &str) -> Result<Vec<(i64, String)>> {
    let parsed = parse_query(text, true);
    if parsed.parts.is_empty() {
        return Ok(Vec::new());
    }
    let Some((clause, scope_values)) = Scope::section(section_id).fts_clause(conn)? else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT p.id, highlight(passages_fts, 0, '{MARK_START}', '{MARK_END}')
         FROM passages_fts CROSS JOIN passages p ON p.id = passages_fts.rowid
         WHERE passages_fts MATCH ?{clause}"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(bound(&parsed.to_fts("OR"), &scope_values)), |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Serialize)]
pub struct RecentSearch {
    pub id: i64,
    pub query: String,
    pub scope: String,
    pub book_id: Option<i64>,
    pub book_title: Option<String>,
    pub result_title: Option<String>,
    pub created_at: String,
}

/// Recent searches, the newest first.
pub fn recent(conn: &Connection) -> Result<Vec<RecentSearch>> {
    let mut statement = conn.prepare(
        "SELECT r.id, r.query, r.scope, r.book_id, b.title, r.result_title, r.created_at
         FROM recent_searches r LEFT JOIN books b ON b.id = r.book_id
         ORDER BY r.created_at DESC, r.id DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RecentSearch {
            id: row.get(0)?,
            query: row.get(1)?,
            scope: row.get(2)?,
            book_id: row.get(3)?,
            book_title: row.get(4)?,
            result_title: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Remembers a search once, however it was capitalised, and keeps only the latest few.
pub fn remember(conn: &Connection, query: &str, scope: ScopeKind, book_id: Option<i64>, result_title: Option<&str>) -> Result<()> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::Invalid("A recent search needs a query.".into()));
    }
    if too_long(query) || result_title.is_some_and(too_long) {
        return Err(Error::Invalid(format!("A search can be at most {MAX_QUERY_CHARS} characters.")));
    }
    if let Some(book_id) = book_id {
        require_book(conn, book_id)?;
    }
    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM recent_searches WHERE lower(query) = lower(?) AND scope = ? AND book_id IS ?",
        rusqlite::params![query, scope.name(), book_id],
    )?;
    transaction.execute(
        "INSERT INTO recent_searches (query, scope, book_id, result_title) VALUES (?, ?, ?, ?)",
        rusqlite::params![query, scope.name(), book_id, result_title],
    )?;
    transaction.execute(
        "DELETE FROM recent_searches WHERE id NOT IN (SELECT id FROM recent_searches ORDER BY created_at DESC, id DESC LIMIT ?)",
        [KEPT_RECENT],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn clear_recent(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM recent_searches", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{knowledge_library, section_id, KnowledgeLibrary};

    fn find(conn: &Connection, text: &str) -> Found {
        search(conn, text, &Scope::default(), false, 50).unwrap()
    }

    fn all_hits(found: &Found) -> Vec<&Hit> {
        found.groups.iter().flat_map(|group| &group.hits).collect()
    }

    fn unmarked(text: &str) -> String {
        text.replace([MARK_START, MARK_END], "")
    }

    // --- Reading what was typed ---

    #[test]
    fn queries_become_quoted_terms() {
        for (text, live, expected) in [
            ("mitochondria energy", false, r#""mitochondria" AND "energy""#),
            (r#""chemical energy" cell"#, false, r#""chemical energy" AND "cell""#),
            ("mitoch", true, r#""mitoch"*"#),
            ("mitochondria ", true, r#""mitochondria""#),
            (r#""chemical ener"#, true, r#""chemical" AND "ener"*"#),
            ("x", true, r#""x""#),
        ] {
            assert_eq!(parse_query(text, live).to_fts("AND"), expected, "{text:?}");
        }
    }

    #[test]
    fn search_syntax_typed_by_the_learner_never_breaks_search() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        for text in ["\"", "AND OR NOT", "energy* NEAR(", "col:umn", "a) OR (b", "\"\" \"\"", "^start", "naïve café", "能量"] {
            assert!(search(&conn, text, &Scope::default(), false, 50).is_ok(), "{text:?}");
            assert!(search(&conn, text, &Scope::default(), true, 50).is_ok(), "{text:?} while typing");
            assert!(highlight_section(&conn, 1, text).is_ok(), "{text:?} marked");
        }
    }

    #[test]
    fn concept_names_are_compared_without_case_or_article() {
        assert_eq!(normalize_name("The Greenhouse-Gas"), "greenhouse gas");
        assert_eq!(normalize_name("An"), "");
    }

    // --- Searching across the library ---

    #[test]
    fn results_are_grouped_by_book_with_their_location() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = find(&conn, "energy");
        let mut titles: Vec<&str> = found.groups.iter().map(|group| group.book_title.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(titles, ["Knowledge Book", "Physics Primer"]);
        let hit = &found.groups.iter().find(|group| group.book_title == "Knowledge Book").unwrap().hits[0];
        assert!(!hit.chapter_title.is_empty() && !hit.section_title.is_empty() && hit.start_page >= 1);
        assert!(hit.snippet.contains(MARK_START) && hit.snippet.contains(MARK_END));
        assert_eq!(found.total, found.groups.iter().map(|group| group.total).sum::<i64>());
    }

    #[test]
    fn a_books_total_is_honest_when_fewer_hits_are_shown() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = search(&conn, "energy", &Scope::default(), false, 2).unwrap();
        assert_eq!(all_hits(&found).len(), 2);
        assert!(found.total > 2);
    }

    #[test]
    fn an_exact_phrase_differs_from_separate_words() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let separate = search(&conn, "energy cell", &Scope::book(knowledge), false, 50).unwrap();
        let phrase = search(&conn, "\"cell needs\"", &Scope::book(knowledge), false, 50).unwrap();
        assert_eq!(phrase.total, 1);
        assert!(separate.total >= phrase.total);
        assert!(unmarked(&phrase.groups[0].hits[0].snippet).contains("a cell needs"));
    }

    #[test]
    fn a_word_still_being_typed_already_matches() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        assert_eq!(find(&conn, "mitoch").total, 0);
        assert!(search(&conn, "mitoch", &Scope::default(), true, 50).unwrap().total >= 1);
    }

    #[test]
    fn other_forms_of_a_word_match() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        assert!(find(&conn, "produces").total >= 1); // the book says "produce"
    }

    #[test]
    fn falls_back_to_any_word_and_says_so() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = find(&conn, "osmosis zzzunmatched");
        assert_eq!(found.match_mode, Some("any"));
        assert!(found.total >= 1);
        assert_eq!(find(&conn, "osmosis").match_mode, Some("all"));
        // One word that matches nothing is not retried: there is nothing looser to try.
        assert_eq!(find(&conn, "zzzunmatched").match_mode, Some("all"));
    }

    #[test]
    fn scopes_narrow_the_results() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let section = section_id(&conn, "Energy in cells");
        let chapter: i64 = conn.query_row("SELECT chapter_id FROM sections WHERE id = ?", [section], |row| row.get(0)).unwrap();
        let everywhere = find(&conn, "energy").total;
        for scope in [Scope::book(knowledge), Scope { kind: ScopeKind::Chapter, chapter_id: Some(chapter), ..Scope::default() }, Scope::section(section)] {
            let found = search(&conn, "energy", &scope, false, 50).unwrap();
            let hits = all_hits(&found);
            assert!(!hits.is_empty() && found.total < everywhere, "{scope:?}");
            assert!(hits.iter().all(|hit| hit.book_id == knowledge));
            if scope.kind != ScopeKind::Book {
                assert!(hits.iter().all(|hit| hit.chapter_id == chapter));
            }
            if scope.kind == ScopeKind::Section {
                assert!(hits.iter().all(|hit| hit.section_id == section));
            }
        }
    }

    #[test]
    fn a_scope_without_its_id_is_refused() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let scope = Scope { kind: ScopeKind::Book, ..Scope::default() };
        assert_eq!(search(&conn, "energy", &scope, false, 50).unwrap_err().to_string(), "A book scope needs a book id.");
        assert!(search(&conn, "", &scope, false, 50).is_err());
    }

    #[test]
    fn a_scope_holding_no_passages_finds_nothing() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = search(&conn, "energy", &Scope::section(987_654), false, 50).unwrap();
        assert!(found.total == 0 && found.groups.is_empty());
    }

    #[test]
    fn books_still_being_processed_are_left_out() {
        let KnowledgeLibrary { conn, physics, .. } = knowledge_library();
        conn.execute("UPDATE books SET status = 'indexing' WHERE id = ?", [physics]).unwrap();
        let found = find(&conn, "energy");
        assert_eq!(found.groups.iter().map(|group| group.book_title.as_str()).collect::<Vec<_>>(), ["Knowledge Book"]);
        assert!(found.concepts.iter().all(|concept| concept.book_title == "Knowledge Book"));
    }

    #[test]
    fn concepts_are_matched_separately() {
        let KnowledgeLibrary { conn, knowledge, physics } = knowledge_library();
        let found = find(&conn, "osmo");
        assert_eq!(found.concepts[0].name.to_lowercase(), "osmosis");
        assert!(found.concepts[0].passage_id > 0 && found.concepts[0].book_title == "Knowledge Book");
        // Signs with a meaning in the comparison are taken as written.
        assert!(find(&conn, "%%").concepts.is_empty() && find(&conn, "__").concepts.is_empty());
        // A concept is only offered where it is mentioned.
        assert!(!search(&conn, "osmosis", &Scope::book(knowledge), false, 50).unwrap().concepts.is_empty());
        assert!(search(&conn, "osmosis", &Scope::book(physics), false, 50).unwrap().concepts.is_empty());
    }

    #[test]
    fn an_empty_query_returns_nothing() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = find(&conn, "   ");
        assert!(found.total == 0 && found.groups.is_empty() && found.match_mode.is_none());
    }

    #[test]
    fn an_overlong_query_is_refused() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        assert!(search(&conn, &"a".repeat(501), &Scope::default(), false, 50).is_err());
        assert!(search(&conn, &"é".repeat(500), &Scope::default(), false, 50).is_ok());
    }

    // --- Text read from scans is searchable like any other text ---

    #[test]
    fn hits_on_scanned_pages_are_flagged() {
        let KnowledgeLibrary { conn, physics, .. } = knowledge_library();
        let found = find(&conn, "energy");
        assert!(all_hits(&found).iter().all(|hit| !hit.from_ocr));
        conn.execute("INSERT INTO pages (book_id, page_number, text, source) SELECT ?, start_page, '', 'ocr' FROM passages WHERE book_id = ? GROUP BY start_page", [physics, physics]).unwrap();
        let found = find(&conn, "energy");
        for hit in all_hits(&found) {
            assert_eq!(hit.from_ocr, hit.book_id == physics);
        }
    }

    // --- Hits inside the open section ---

    #[test]
    fn every_query_word_is_marked_in_the_open_section() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let hit_section = all_hits(&find(&conn, "mitochondria energy"))[0].section_id;
        let marked = highlight_section(&conn, hit_section, "mitochondria energy").unwrap();
        let (passage, text) = &marked[0];
        assert!(text.contains(&format!("{MARK_START}Mitochondria{MARK_END}")) && text.contains(&format!("{MARK_START}energy{MARK_END}")));
        let original: String = conn.query_row("SELECT text FROM passages WHERE id = ?", [passage], |row| row.get(0)).unwrap();
        assert_eq!(unmarked(text), original);
    }

    #[test]
    fn what_live_search_found_is_what_gets_marked() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        let found = search(&conn, "mitoch", &Scope::default(), true, 50).unwrap();
        let marked = highlight_section(&conn, all_hits(&found)[0].section_id, "mitoch").unwrap();
        assert!(marked[0].1.contains(&format!("{MARK_START}Mitochondria{MARK_END}")));
        assert!(highlight_section(&conn, 987_654, "energy").unwrap().is_empty());
        assert!(highlight_section(&conn, 1, "  ").unwrap().is_empty());
    }

    // --- Recent searches ---

    #[test]
    fn recent_searches_are_newest_first_and_kept_once() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        for query in ["osmosis", "gravity", "OSMOSIS"] {
            remember(&conn, query, ScopeKind::Book, Some(knowledge), Some("Cells")).unwrap();
        }
        let kept = recent(&conn).unwrap();
        assert_eq!(kept.iter().map(|item| item.query.as_str()).collect::<Vec<_>>(), ["OSMOSIS", "gravity"]);
        assert_eq!(kept[0].book_title.as_deref(), Some("Knowledge Book"));
        assert_eq!((kept[0].scope.as_str(), kept[0].result_title.as_deref()), ("book", Some("Cells")));
        // The same words in another scope are a different search.
        remember(&conn, "osmosis", ScopeKind::All, None, None).unwrap();
        assert_eq!(recent(&conn).unwrap().len(), 3);
    }

    #[test]
    fn recent_searches_are_bounded_and_can_be_cleared() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        for n in 0..25 {
            remember(&conn, &format!("query {n}"), ScopeKind::All, None, None).unwrap();
        }
        let kept = recent(&conn).unwrap();
        assert_eq!((kept.len(), kept[0].query.as_str()), (20, "query 24"));
        clear_recent(&conn).unwrap();
        assert!(recent(&conn).unwrap().is_empty());
    }

    #[test]
    fn recent_searches_go_with_their_book() {
        let KnowledgeLibrary { conn, physics, .. } = knowledge_library();
        remember(&conn, "energy", ScopeKind::Book, Some(physics), None).unwrap();
        conn.execute("DELETE FROM books WHERE id = ?", [physics]).unwrap();
        assert!(recent(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_recent_search_must_be_well_formed() {
        let KnowledgeLibrary { conn, .. } = knowledge_library();
        assert!(matches!(remember(&conn, "  ", ScopeKind::All, None, None), Err(Error::Invalid(_))));
        assert!(matches!(remember(&conn, "x", ScopeKind::All, Some(999), None), Err(Error::NotFound("Book"))));
        assert!(remember(&conn, &"x".repeat(501), ScopeKind::All, None, None).is_err());
        assert!(serde_json::from_str::<ScopeKind>("\"galaxy\"").is_err());
        assert!(recent(&conn).unwrap().is_empty());
    }
}
