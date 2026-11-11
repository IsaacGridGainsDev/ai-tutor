//! Visuals drawn from what an uploaded book actually says.
//!
//! Three kinds are found with no model: a map of the concepts in a part of the book, processes the
//! text sets out step by step, and series of figures the text states. Every item carries the
//! passage it came from, and step wording and figures are the book's own.
//!
//! Diagrams planned by the local model are stored in the same database; this module lists and
//! removes them, and the model call itself moves here with the rest of the model client.

use std::sync::LazyLock;

use fancy_regex::Regex;
use rusqlite::{params_from_iter, Connection};
use serde::Serialize;
use serde_json::Value;

use crate::{require_book, round1, Error, Origin, Result};

const MAX_NODES: usize = 24;
const MAX_EDGES: usize = 60;
const MIN_STEPS: usize = 3;
const MAX_STEPS: usize = 12;
const MIN_SERIES: usize = 3;
const LABEL_WORDS: usize = 3;
const MAX_FOUND: usize = 12;

/// Which part of a book to draw from: all of it, one chapter or one section.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scope {
    pub chapter_id: Option<i64>,
    pub section_id: Option<i64>,
}

impl Scope {
    /// A filter on passages `p`, and its values. A section takes precedence over a chapter.
    fn clause(self, book_id: i64) -> (&'static str, Vec<i64>) {
        match (self.section_id, self.chapter_id) {
            (Some(section), _) => ("p.book_id = ? AND p.section_id = ?", vec![book_id, section]),
            (None, Some(chapter)) => ("p.book_id = ? AND p.chapter_id = ?", vec![book_id, chapter]),
            (None, None) => ("p.book_id = ?", vec![book_id]),
        }
    }
}

struct Passage {
    origin: Origin,
    text: String,
}

fn passages(conn: &Connection, clause: &str, values: &[i64]) -> Result<Vec<Passage>> {
    let mut statement = conn.prepare(&format!(
        "SELECT p.id, p.section_id, s.title, p.start_page, p.end_page, p.text
         FROM passages p JOIN sections s ON s.id = p.section_id
         WHERE {clause} ORDER BY p.start_page, p.id"
    ))?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        Ok(Passage {
            origin: Origin { passage_id: row.get(0)?, section_id: row.get(1)?, section_title: row.get(2)?, start_page: row.get(3)?, end_page: row.get(4)? },
            text: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// --- Concept map ---

#[derive(Clone, Debug, Serialize)]
pub struct ConceptNode {
    pub id: i64,
    pub name: String,
    pub definition: Option<String>,
    pub importance: f64,
    /// Passages in scope that mention it.
    pub mentions: i64,
    #[serde(flatten)]
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConceptEdge {
    pub source: i64,
    pub target: i64,
    /// Passages in scope where both concepts appear.
    pub weight: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConceptMap {
    pub nodes: Vec<ConceptNode>,
    pub edges: Vec<ConceptEdge>,
}

/// The most important concepts in scope, linked when they appear in the same passage.
fn concept_map(conn: &Connection, clause: &str, values: &[i64]) -> Result<Option<ConceptMap>> {
    let mut statement = conn.prepare(&format!(
        "SELECT c.id, c.name, c.definition, c.importance, COUNT(*) AS mentions, MIN(p.id) AS passage_id
         FROM concepts c JOIN passage_concepts pc ON pc.concept_id = c.id JOIN passages p ON p.id = pc.passage_id
         WHERE {clause} GROUP BY c.id ORDER BY c.importance DESC, mentions DESC, c.name LIMIT {MAX_NODES}"
    ))?;
    let found: Vec<(i64, String, Option<String>, f64, i64, i64)> = statement
        .query_map(params_from_iter(values), |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if found.len() < 2 {
        return Ok(None);
    }
    let mut nodes = Vec::with_capacity(found.len());
    for (id, name, definition, importance, mentions, passage_id) in found {
        let origin = conn.query_row(
            "SELECT p.id, p.section_id, s.title, p.start_page, p.end_page FROM passages p JOIN sections s ON s.id = p.section_id WHERE p.id = ?",
            [passage_id],
            |row| Ok(Origin { passage_id: row.get(0)?, section_id: row.get(1)?, section_title: row.get(2)?, start_page: row.get(3)?, end_page: row.get(4)? }),
        )?;
        nodes.push(ConceptNode { id, name, definition, importance, mentions, origin });
    }
    let marks = vec!["?"; nodes.len()].join(",");
    let ids: Vec<i64> = nodes.iter().map(|node| node.id).collect();
    let mut statement = conn.prepare(&format!(
        "SELECT a.concept_id, b.concept_id, COUNT(*) AS weight
         FROM passage_concepts a
         JOIN passage_concepts b ON b.passage_id = a.passage_id AND a.concept_id < b.concept_id
         JOIN passages p ON p.id = a.passage_id
         WHERE {clause} AND a.concept_id IN ({marks}) AND b.concept_id IN ({marks})
         GROUP BY a.concept_id, b.concept_id ORDER BY weight DESC, a.concept_id, b.concept_id LIMIT {MAX_EDGES}"
    ))?;
    let edges = statement
        .query_map(params_from_iter(values.iter().chain(&ids).chain(&ids)), |row| {
            Ok(ConceptEdge { source: row.get(0)?, target: row.get(1)?, weight: row.get(2)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(ConceptMap { nodes, edges }))
}

// --- Processes the text sets out in steps ---

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Step {
    /// The opening words of the step, for the diagram.
    pub label: String,
    /// The step in the book's own words.
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Process {
    #[serde(flatten)]
    pub origin: Origin,
    /// How the text marked the steps: `numbered`, `ordinal` or `arrows`.
    pub style: &'static str,
    pub steps: Vec<Step>,
}

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:(?<=\s)|^)\(?(\d{1,2})[.)]\s+(?=[A-Z"“(])"#).expect("numbered pattern"));
static SENTENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?<=[.!?])["”)]?\s+(?=[A-Z0-9"“(])"#).expect("sentence pattern"));
static ORDINAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(First(?:ly)?|Second(?:ly)?|Third(?:ly)?|Fourth(?:ly)?|Fifth(?:ly)?|Then|Next|After that|Afterwards?|Subsequently|Finally|Lastly|Last)\b[,:]?\s+")
        .expect("ordinal pattern")
});
const ARROW: &str = r"\s*(?:→|->|⟶|➔|=>)\s*";
static ARROW_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(ARROW).expect("arrow pattern"));
static ARROW_CHAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"[^.;\n→>]{{2,60}}(?:{ARROW}[^.;\n→>=-]{{2,60}}){{2,}}")).expect("arrow chain pattern"));

fn split<'t>(pattern: &Regex, text: &'t str) -> Vec<&'t str> {
    let mut pieces = Vec::new();
    let mut last = 0;
    for found in pattern.find_iter(text).flatten() {
        pieces.push(&text[last..found.start()]);
        last = found.end();
    }
    pieces.push(&text[last..]);
    pieces
}

fn label(text: &str) -> String {
    const WORDS: usize = 7;
    let parts: Vec<&str> = text.trim().trim_end_matches('.').split_whitespace().collect();
    let mut label = parts.iter().take(WORDS).copied().collect::<Vec<_>>().join(" ");
    if parts.len() > WORDS {
        label.push('…');
    }
    label
}

pub(crate) fn numbered_steps(text: &str) -> Vec<Step> {
    // Each mark: where it starts, where its step begins, and its number.
    let marks: Vec<(usize, usize, usize)> = NUMBERED
        .captures_iter(text)
        .flatten()
        .filter_map(|found| {
            let whole = found.get(0)?;
            Some((whole.start(), whole.end(), found.get(1)?.as_str().parse().ok()?))
        })
        .collect();
    // The longest run that counts up from 1.
    let mut best: Vec<(usize, usize, usize)> = Vec::new();
    for (start, first) in marks.iter().enumerate() {
        if first.2 != 1 {
            continue;
        }
        let mut run = vec![*first];
        for mark in &marks[start + 1..] {
            if mark.2 == run.len() + 1 {
                run.push(*mark);
            }
        }
        if run.len() > best.len() {
            best = run;
        }
    }
    if best.len() < MIN_STEPS {
        return Vec::new();
    }
    best.iter()
        .enumerate()
        .take(MAX_STEPS)
        .map(|(index, mark)| {
            let end = best.get(index + 1).map_or(text.len(), |next| next.0);
            let mut body = text[mark.1..end].split("\n\n").next().unwrap_or("").trim();
            if index + 1 == best.len() {
                // Nothing marks where the last step ends, so it stops at its first sentence.
                body = split(&SENTENCE, body)[0];
            }
            Step { label: label(body), text: body.to_string() }
        })
        .collect()
}

pub(crate) fn ordinal_steps(text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut started = false;
    for paragraph in text.split("\n\n") {
        for sentence in split(&SENTENCE, paragraph).into_iter().map(str::trim) {
            let Ok(Some(marker)) = ORDINAL.find(sentence) else { continue };
            if marker.as_str().starts_with("First") {
                steps.clear();
                started = true;
            }
            if started {
                steps.push(Step { label: label(&sentence[marker.end()..]), text: sentence.to_string() });
            }
        }
    }
    if steps.len() < MIN_STEPS {
        return Vec::new();
    }
    steps.truncate(MAX_STEPS);
    steps
}

pub(crate) fn arrow_steps(text: &str) -> Vec<Step> {
    let Ok(Some(chain)) = ARROW_CHAIN.find(text) else { return Vec::new() };
    let trim = |part: &str| part.trim_matches([' ', ':', ',']).to_string();
    let mut parts: Vec<String> = split(&ARROW_SPLIT, chain.as_str()).into_iter().map(trim).collect();
    // "In short: evaporation" names the sequence before its first step.
    parts[0] = parts[0].rsplit(':').next().unwrap_or("").trim().to_string();
    parts.into_iter()
        .filter(|part| !part.is_empty())
        .take(MAX_STEPS)
        .map(|part| Step { label: part, text: chain.as_str().trim().to_string() })
        .collect()
}

fn processes(passages: &[Passage]) -> Vec<Process> {
    let finders: [(&'static str, fn(&str) -> Vec<Step>); 3] = [("numbered", numbered_steps), ("ordinal", ordinal_steps), ("arrows", arrow_steps)];
    let mut found = Vec::new();
    for passage in passages {
        for (style, finder) in finders {
            let steps = finder(&passage.text);
            if steps.len() >= MIN_STEPS {
                found.push(Process { origin: passage.origin.clone(), style, steps });
                break;
            }
        }
        if found.len() == MAX_FOUND {
            break;
        }
    }
    found
}

// --- Series of figures the text states ---

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Datum {
    pub label: String,
    pub value: f64,
    /// The sentence the value was read from.
    pub quote: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Chart {
    #[serde(flatten)]
    pub origin: Origin,
    /// `bar` for a list of percentages, `line` for values by year.
    pub kind: &'static str,
    pub unit: &'static str,
    pub items: Vec<Datum>,
    pub total: Option<f64>,
    /// The percentages add up to a whole.
    pub whole: bool,
}

static PERCENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?<![\w.])(\d{1,3}(?:\.\d+)?)\s*(?:%|per\s?cent\b)").expect("percent pattern"));
static WORD: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[A-Za-z][A-Za-z'’-]*").expect("word pattern"));
static YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?<![\w.])(1[5-9]\d\d|20\d\d)[:\s]\s*(\d[\d,]*(?:\.\d+)?)(?![\w%]|\.\d)").expect("year pattern"));
const SKIP_BEFORE: [&str; 17] = [
    "is", "are", "was", "were", "at", "about", "of", "to", "only", "nearly", "around", "roughly", "almost", "approximately", "just", "for", "by",
];
const SKIP_BEFORE_MORE: [&str; 2] = ["makes", "up"];
const STOP: [&str; 30] = [
    "the", "a", "an", "and", "or", "in", "with", "than", "that", "this", "its", "as", "from", "over", "under", "while", "but", "which",
    "has", "have", "had", "be", "been", "it", "on", "some", "remaining", "other", "another", "rest",
];

fn skipped_before(word: &str) -> bool {
    let word = word.to_lowercase();
    SKIP_BEFORE.contains(&word.as_str()) || SKIP_BEFORE_MORE.contains(&word.as_str())
}

fn stop_word(word: &str) -> bool {
    skipped_before(word) || STOP.contains(&word.to_lowercase().as_str())
}

/// Splits where a label cannot continue: punctuation, digits, a percent sign or a line break.
fn label_breaks(c: char) -> bool {
    matches!(c, '.' | ';' | ',' | ':' | '(' | ')' | '%' | '\n') || c.is_numeric()
}

fn sentence_around(text: &str, start: usize, end: usize) -> String {
    let left = [text[..start].rfind(". "), text[..start].rfind('\n')].into_iter().flatten().max().map_or(0, |position| position + 1);
    let right = [text[end..].find(". "), text[end..].find('\n')].into_iter().flatten().min().map_or(text.len(), |position| end + position + 1);
    text[left..right.min(text.len())].trim().to_string()
}

/// Up to three content words directly before a number, such as "oxygen" in "oxygen is 21%".
fn label_before(text: &str, position: usize) -> String {
    let from = text[..position].char_indices().rev().nth(59).map_or(0, |(index, _)| index);
    let segment = text[from..position].rsplit(label_breaks).next().unwrap_or("");
    let mut words: Vec<&str> = WORD.find_iter(segment).map(|word| word.as_str()).collect();
    while words.last().is_some_and(|word| skipped_before(word)) {
        words.pop();
    }
    let mut label: Vec<&str> = Vec::new();
    for word in words.into_iter().rev() {
        if stop_word(word) || label.len() == LABEL_WORDS {
            break;
        }
        label.insert(0, word);
    }
    label.join(" ")
}

/// Up to three content words directly after a percentage, such as "nitrogen" in "78% nitrogen".
fn label_after(text: &str, position: usize) -> String {
    let to = text[position..].char_indices().nth(60).map_or(text.len(), |(index, _)| position + index);
    let segment = text[position..to].split(label_breaks).next().unwrap_or("");
    let mut words: Vec<&str> = WORD.find_iter(segment).map(|word| word.as_str()).collect();
    if words.first().is_some_and(|word| word.eq_ignore_ascii_case("of")) {
        words.remove(0);
    }
    let mut label: Vec<&str> = Vec::new();
    for word in words {
        if stop_word(word) || label.len() == LABEL_WORDS {
            break;
        }
        label.push(word);
    }
    label.join(" ")
}

pub(crate) fn percent_series(text: &str) -> Vec<Datum> {
    // Each mark: where it starts and ends, and its value.
    let marks: Vec<(usize, usize, f64)> = PERCENT
        .captures_iter(text)
        .flatten()
        .filter_map(|found| {
            let whole = found.get(0)?;
            let value: f64 = found.get(1)?.as_str().parse().ok()?;
            (value <= 100.0).then_some((whole.start(), whole.end(), value))
        })
        .collect();
    if marks.len() < MIN_SERIES {
        return Vec::new();
    }
    // A passage is read one way throughout: labels after the figures, or before them.
    for after in [true, false] {
        let mut items: Vec<Datum> = Vec::new();
        for &(start, end, value) in &marks {
            let label = if after { label_after(text, end) } else { label_before(text, start) };
            if !label.is_empty() && !items.iter().any(|item| item.label.to_lowercase() == label.to_lowercase()) {
                items.push(Datum { label, value, quote: sentence_around(text, start, end) });
            }
        }
        if items.len() >= MIN_SERIES && items.len() as f64 >= marks.len() as f64 * 0.6 {
            return items;
        }
    }
    Vec::new()
}

pub(crate) fn year_series(text: &str) -> Vec<Datum> {
    let mut items = Vec::new();
    let mut last = 0;
    for found in YEAR.captures_iter(text).flatten() {
        let (Some(whole), Some(year), Some(value)) = (found.get(0), found.get(1), found.get(2)) else { continue };
        let (Ok(number), Ok(value)) = (year.as_str().parse::<i32>(), value.as_str().replace(',', "").parse::<f64>()) else { continue };
        if number <= last {
            continue;
        }
        last = number;
        items.push(Datum { label: year.as_str().to_string(), value, quote: sentence_around(text, whole.start(), whole.end()) });
    }
    if items.len() > MIN_SERIES { items } else { Vec::new() }
}

fn charts(passages: &[Passage]) -> Vec<Chart> {
    let mut found = Vec::new();
    for passage in passages {
        let percents = percent_series(&passage.text);
        if !percents.is_empty() {
            let total = round1(percents.iter().map(|item| item.value).sum());
            found.push(Chart { origin: passage.origin.clone(), kind: "bar", unit: "%", items: percents, total: Some(total), whole: (99.0..=101.0).contains(&total) });
        }
        let years = year_series(&passage.text);
        if !years.is_empty() {
            found.push(Chart { origin: passage.origin.clone(), kind: "line", unit: "", items: years, total: None, whole: false });
        }
        if found.len() >= MAX_FOUND {
            break;
        }
    }
    found.truncate(MAX_FOUND);
    found
}

// --- Everything that can be drawn for a part of a book ---

#[derive(Debug, Serialize)]
pub struct Visuals {
    pub passage_count: usize,
    pub concept_map: Option<ConceptMap>,
    pub processes: Vec<Process>,
    pub charts: Vec<Chart>,
    /// Diagrams of sections in scope that the local model planned earlier.
    pub diagrams: Vec<Value>,
}

/// A stored diagram as the window expects it: its record, with its nodes and links alongside.
pub(crate) fn diagram_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let data: String = row.get("data")?;
    let mut view = serde_json::from_str::<Value>(&data).ok().filter(Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    let fields = view.as_object_mut().expect("an object");
    fields.insert("id".into(), row.get::<_, i64>("id")?.into());
    fields.insert("section_id".into(), row.get::<_, i64>("section_id")?.into());
    for key in ["kind", "title", "model", "created_at"] {
        fields.insert(key.into(), row.get::<_, String>(key)?.into());
    }
    Ok(view)
}

fn diagrams(conn: &Connection, book_id: i64, scope: Scope) -> Result<Vec<Value>> {
    let (filter, values) = match (scope.section_id, scope.chapter_id) {
        (Some(section), _) => ("AND section_id = ?", vec![book_id, section]),
        (None, Some(chapter)) => ("AND section_id IN (SELECT id FROM sections WHERE chapter_id = ?)", vec![book_id, chapter]),
        (None, None) => ("", vec![book_id]),
    };
    let mut statement = conn.prepare(&format!("SELECT * FROM diagrams WHERE book_id = ? {filter} ORDER BY created_at DESC, id DESC LIMIT 20"))?;
    let rows = statement.query_map(params_from_iter(values), diagram_view)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn analyse(conn: &Connection, book_id: i64, scope: Scope) -> Result<Visuals> {
    require_book(conn, book_id)?;
    let (clause, values) = scope.clause(book_id);
    let passages = passages(conn, clause, &values)?;
    Ok(Visuals {
        passage_count: passages.len(),
        concept_map: concept_map(conn, clause, &values)?,
        processes: processes(&passages),
        charts: charts(&passages),
        diagrams: diagrams(conn, book_id, scope)?,
    })
}

pub fn delete_diagram(conn: &Connection, id: i64) -> Result<()> {
    match conn.execute("DELETE FROM diagrams WHERE id = ?", [id])? {
        0 => Err(Error::NotFound("Diagram")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{book, library, section_id};

    const TREATING: &str = "Water treatment follows a fixed order. 1. Screening removes leaves and large debris. \
        2. Coagulation makes fine particles clump together. 3. Sedimentation lets the clumps settle. \
        4. Filtration passes the water through sand. Each stage depends on the one before it.";
    const CYCLE: &str = "First, the sun heats the sea and water evaporates. Then the vapour rises and cools into cloud. \
        Finally, the water falls as rain and returns to the sea.";
    const SHORT: &str = "In short: evaporation → condensation → precipitation → collection.";
    const AIR: &str = "Dry air is 78% nitrogen, 21% oxygen and 0.9% argon, with traces of other gases.";
    const RECORD: &str = "Carbon dioxide at the station, in parts per million: 1960 317, 1980 339, 2000 369, 2020 414.";

    fn texts(steps: &[Step]) -> Vec<&str> {
        steps.iter().map(|step| step.text.as_str()).collect()
    }

    fn labels(steps: &[Step]) -> Vec<&str> {
        steps.iter().map(|step| step.label.as_str()).collect()
    }

    fn pairs(items: &[Datum]) -> Vec<(&str, f64)> {
        items.iter().map(|item| (item.label.as_str(), item.value)).collect()
    }

    /// The book the Python tests used, as its passages stood after extraction.
    fn content_book(conn: &Connection) -> i64 {
        let id = book(conn, "Water and Air", &[
            ("Treating water", &[TREATING, "The process by which fine particles clump together is called coagulation."]),
            ("The water cycle", &[CYCLE, SHORT]),
            ("What air is made of", &[AIR, "The layer of gases around the planet is called the atmosphere."]),
            ("A rising record", &[RECORD, "A gas that traps heat in the atmosphere is called a greenhouse gas."]),
            ("Glossary", &["atmosphere: The layer of gases. coagulation: The clumping of fine particles. greenhouse gas: A gas that traps heat."]),
        ]);
        // Concepts as the ingest step records them: each tied to the passages that mention it.
        for (name, definition, importance) in [
            ("coagulation", Some("The clumping of fine particles so they can settle."), 3.0),
            ("atmosphere", Some("The layer of gases around the planet."), 2.0),
            ("greenhouse gas", None, 1.0),
        ] {
            conn.execute(
                "INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![id, name, name, definition, importance],
            ).unwrap();
            let concept = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO passage_concepts (passage_id, concept_id) SELECT id, ? FROM passages WHERE book_id = ? AND text LIKE '%' || ? || '%'",
                rusqlite::params![concept, id, name],
            ).unwrap();
        }
        id
    }

    fn section_scope(conn: &Connection, title: &str) -> Scope {
        Scope { section_id: Some(section_id(conn, title)), chapter_id: None }
    }

    // --- Finding steps and figures in text ---

    #[test]
    fn numbered_steps_keep_the_books_wording() {
        let steps = numbered_steps("Do this. 1. Screening removes debris. 2. Coagulation clumps particles. 3. Filtration uses sand. More follows.");
        assert_eq!(texts(&steps), ["Screening removes debris.", "Coagulation clumps particles.", "Filtration uses sand."]);
    }

    #[test]
    fn ordinal_steps_start_at_first() {
        let steps = ordinal_steps("Then nothing. First, heat the water. It takes time. Then the vapour rises. Finally, rain falls.");
        assert_eq!(labels(&steps), ["heat the water", "the vapour rises", "rain falls"]);
        assert_eq!(steps[0].text, "First, heat the water.");
    }

    #[test]
    fn arrow_chain_becomes_steps() {
        assert_eq!(labels(&arrow_steps("In short: evaporation → condensation -> precipitation.")), ["evaporation", "condensation", "precipitation"]);
    }

    #[test]
    fn text_without_a_process_yields_no_steps() {
        for text in [
            "Figure 2. The plant. See section 1. Results follow in 3. Tables.", // numbers that are not a 1, 2, 3 run
            "Then it rained. Next we left. Finally we slept.",                   // no "First"
            "First, we open. Then we close.",                                    // too few steps
            "x -> y",                                                            // too short a chain
        ] {
            assert!(numbered_steps(text).is_empty() && ordinal_steps(text).is_empty() && arrow_steps(text).len() < MIN_STEPS, "{text:?}");
        }
    }

    #[test]
    fn percentages_are_read_with_labels_on_either_side() {
        let after = percent_series("Dry air is 78% nitrogen, 21% oxygen and 0.9% argon.");
        assert_eq!(pairs(&after), [("nitrogen", 78.0), ("oxygen", 21.0), ("argon", 0.9)]);
        assert_eq!(after[0].quote, "Dry air is 78% nitrogen, 21% oxygen and 0.9% argon.");
        let before = percent_series("By mass the crust is oxygen at 46%, silicon at 28 per cent, and aluminium is about 8%.");
        assert_eq!(pairs(&before), [("oxygen", 46.0), ("silicon", 28.0), ("aluminium", 8.0)]);
    }

    #[test]
    fn years_with_values_become_a_series() {
        let series = year_series("Parts per million: 1960 317, 1980 339, 2000 369, 2020 414.");
        assert_eq!(pairs(&series), [("1960", 317.0), ("1980", 339.0), ("2000", 369.0), ("2020", 414.0)]);
    }

    #[test]
    fn text_without_a_series_yields_no_chart() {
        for text in [
            "Sales rose 5% and then 7%.",                  // no labels, and too few
            "It was founded in 1990 and rebuilt in 2005.", // years with no values
            "About 40% of 30% of 20% remained.",           // nothing nameable
        ] {
            assert!(percent_series(text).is_empty() && year_series(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn text_in_other_scripts_is_read_without_error() {
        let text = "Влажность: азот — 78% воздуха, кислород — 21% воздуха, аргон — 0,9% воздуха. «Первый» шаг → второй → третий.";
        let _ = (percent_series(text), year_series(text), numbered_steps(text), ordinal_steps(text), arrow_steps(text));
    }

    // --- What can be drawn from an uploaded book ---

    #[test]
    fn a_section_yields_the_process_it_describes() {
        let conn = library();
        let book = content_book(&conn);
        let visuals = analyse(&conn, book, section_scope(&conn, "Treating water")).unwrap();
        assert_eq!(visuals.processes.len(), 1);
        let process = &visuals.processes[0];
        assert_eq!((process.style, process.origin.section_title.as_str(), process.origin.start_page), ("numbered", "Treating water", 2));
        let first_words: Vec<&str> = process.steps.iter().map(|step| step.label.split(' ').next().unwrap()).collect();
        assert_eq!(first_words, ["Screening", "Coagulation", "Sedimentation", "Filtration"]);
        assert_eq!(process.steps[3].text, "Filtration passes the water through sand.");
        assert!(visuals.charts.is_empty());
    }

    #[test]
    fn ordinal_and_arrow_processes_are_both_found() {
        let conn = library();
        let book = content_book(&conn);
        let visuals = analyse(&conn, book, section_scope(&conn, "The water cycle")).unwrap();
        assert_eq!(visuals.processes.iter().map(|process| process.style).collect::<Vec<_>>(), ["ordinal", "arrows"]);
        assert_eq!(labels(&visuals.processes[1].steps), ["evaporation", "condensation", "precipitation", "collection"]);
    }

    #[test]
    fn figures_stated_in_the_text_become_charts() {
        let conn = library();
        let book = content_book(&conn);
        let charts = analyse(&conn, book, Scope::default()).unwrap().charts;
        let bar = charts.iter().find(|chart| chart.kind == "bar").unwrap();
        assert_eq!(pairs(&bar.items), [("nitrogen", 78.0), ("oxygen", 21.0), ("argon", 0.9)]);
        assert!(bar.whole && bar.total == Some(99.9) && bar.items[0].quote.contains("78% nitrogen"));
        let line = charts.iter().find(|chart| chart.kind == "line").unwrap();
        assert_eq!(line.items.iter().map(|item| item.value).collect::<Vec<_>>(), [317.0, 339.0, 369.0, 414.0]);
        assert_eq!(line.origin.section_title, "A rising record");
    }

    #[test]
    fn concept_map_comes_from_the_books_own_concepts() {
        let conn = library();
        let book = content_book(&conn);
        let map = analyse(&conn, book, Scope::default()).unwrap().concept_map.unwrap();
        assert_eq!(map.nodes.iter().map(|node| node.name.as_str()).collect::<Vec<_>>(), ["coagulation", "atmosphere", "greenhouse gas"]);
        assert_eq!(map.nodes[0].mentions, 3); // the steps, its definition and the glossary
        assert_eq!(map.nodes[0].origin.section_title, "Treating water");
        let ids: Vec<i64> = map.nodes.iter().map(|node| node.id).collect();
        assert!(!map.edges.is_empty() && map.edges.iter().all(|edge| ids.contains(&edge.source) && ids.contains(&edge.target)));
        // Atmosphere and greenhouse gas share two passages; that link is the strongest.
        assert_eq!(map.edges[0], ConceptEdge { source: ids[1], target: ids[2], weight: 2 });
    }

    #[test]
    fn scope_narrows_what_is_found() {
        let conn = library();
        let book = content_book(&conn);
        let whole = analyse(&conn, book, Scope::default()).unwrap();
        let part = analyse(&conn, book, section_scope(&conn, "What air is made of")).unwrap();
        assert!(whole.processes.len() >= 3 && part.processes.is_empty());
        assert!(part.passage_count < whole.passage_count);
        assert!(part.concept_map.is_none(), "one concept is not a map");
        let chapter: i64 = conn.query_row("SELECT id FROM chapters WHERE book_id = ?", [book], |row| row.get(0)).unwrap();
        assert_eq!(analyse(&conn, book, Scope { chapter_id: Some(chapter), section_id: None }).unwrap().passage_count, whole.passage_count);
        assert!(matches!(analyse(&conn, 9999, Scope::default()), Err(Error::NotFound("Book"))));
    }

    #[test]
    fn the_result_has_the_shape_the_window_expects() {
        let conn = library();
        let book = content_book(&conn);
        let visuals = serde_json::to_value(analyse(&conn, book, Scope::default()).unwrap()).unwrap();
        for key in ["passage_id", "section_id", "section_title", "start_page", "end_page", "style", "steps"] {
            assert!(visuals["processes"][0].get(key).is_some(), "process is missing {key}");
        }
        for key in ["passage_id", "section_title", "kind", "unit", "items", "total", "whole"] {
            assert!(visuals["charts"][0].get(key).is_some(), "chart is missing {key}");
        }
        for key in ["id", "name", "definition", "importance", "mentions", "passage_id", "section_title", "start_page"] {
            assert!(visuals["concept_map"]["nodes"][0].get(key).is_some(), "concept is missing {key}");
        }
    }

    #[test]
    fn saved_model_diagrams_are_listed_for_their_section_and_can_be_removed() {
        let conn = library();
        let book = content_book(&conn);
        let section = section_id(&conn, "Treating water");
        let data = r#"{"nodes": [{"id": "n1", "label": "Screening", "quote": "Screening removes leaves", "passage_id": 1}], "edges": [], "dropped": 1}"#;
        conn.execute(
            "INSERT INTO diagrams (book_id, section_id, kind, title, data, model) VALUES (?, ?, 'process', 'How water is treated', ?, 'local:1')",
            rusqlite::params![book, section, data],
        ).unwrap();
        let id = conn.last_insert_rowid();

        let listed = analyse(&conn, book, section_scope(&conn, "Treating water")).unwrap().diagrams;
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0]["id"].as_i64(), listed[0]["kind"].as_str(), listed[0]["dropped"].as_i64()), (Some(id), Some("process"), Some(1)));
        assert_eq!(listed[0]["nodes"][0]["label"], "Screening");
        assert_eq!(analyse(&conn, book, Scope::default()).unwrap().diagrams.len(), 1);
        assert!(analyse(&conn, book, section_scope(&conn, "What air is made of")).unwrap().diagrams.is_empty());

        delete_diagram(&conn, id).unwrap();
        assert!(matches!(delete_diagram(&conn, id), Err(Error::NotFound("Diagram"))));
    }
}
