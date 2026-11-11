//! A diagram of a section planned by the local model, kept only where the text supports it.

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::concepts::excerpt;
use crate::content::diagram_view;
use crate::grounding::{field, quote_appears_in};
use crate::llm::{complete, Llm, Message};
use crate::{prompts, Error, ModelProblem, Origin, Result};

const MAX_EXCERPT_CHARS: usize = 5000;
const KINDS: [&str; 4] = ["process", "hierarchy", "comparison", "cause_effect"];

fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn failed(message: &str) -> Error {
    Error::Model(ModelProblem::Failed, message.into())
}

/// Asks the model to plan a diagram of a section, keeps the parts whose quotations are real,
/// and saves it with the book.
pub fn draw_section(conn: &Connection, llm: &dyn Llm, section_id: i64, model: &str, options: &Value) -> Result<Value> {
    let section: Option<(String, i64)> = conn
        .query_row("SELECT s.title, c.book_id FROM sections s JOIN chapters c ON c.id = s.chapter_id WHERE s.id = ?", [section_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((section_title, book_id)) = section else { return Err(Error::NotFound("Section")) };
    let mut statement = conn.prepare("SELECT id, start_page, end_page, text FROM passages WHERE section_id = ? ORDER BY start_page, id")?;
    let passages: Vec<(Origin, String)> = statement
        .query_map([section_id], |row| {
            Ok((Origin { passage_id: row.get(0)?, section_id, section_title: section_title.clone(), start_page: row.get(1)?, end_page: row.get(2)? }, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if passages.is_empty() {
        return Err(failed("This section has no text to draw from."));
    }
    let (sent, included) = excerpt(passages, |(_, text)| text, MAX_EXCERPT_CHARS);

    let messages = [Message::system(prompts::DIAGRAM_SYSTEM), Message::user(prompts::diagram_user(&section_title, &sent))];
    let content = complete(llm, model, &messages, &prompts::diagram_schema(), options)?;
    let unreadable = || failed("The model returned a diagram that could not be read. Please try again.");
    let plan: Value = serde_json::from_str(&content).map_err(|_| unreadable())?;
    let raw_nodes = plan["nodes"].as_array().ok_or_else(unreadable)?;

    let mut nodes: Vec<Value> = Vec::new();
    let mut dropped = 0;
    for item in raw_nodes {
        let (key, label, quote) = (field(item, "id"), field(item, "label"), field(item, "quote"));
        let source = included.iter().find(|(_, text)| quote_appears_in(&quote, text));
        match source {
            Some((origin, _)) if !key.is_empty() && !label.is_empty() && nodes.iter().all(|node| node["id"] != key.as_str()) => {
                let mut node = serde_json::to_value(origin).expect("an origin is plain data");
                node["id"] = json!(key);
                node["label"] = json!(cut(&label, 80));
                node["quote"] = json!(quote);
                nodes.push(node);
            }
            // Unsupported, unnamed or repeated: left out, and counted so the learner is told.
            _ => dropped += 1,
        }
    }
    if nodes.len() < 2 {
        return Err(failed("The model's diagram could not be matched to the text of this section, so nothing was drawn."));
    }

    let kept = |id: &str| nodes.iter().any(|node| node["id"] == id);
    let edges: Vec<Value> = plan["edges"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|item| {
        let (source, target) = (field(item, "from"), field(item, "to"));
        (kept(&source) && kept(&target) && source != target).then(|| json!({ "source": source, "target": target, "label": cut(&field(item, "label"), 60) }))
    }).collect();

    let kind = plan["kind"].as_str().filter(|kind| KINDS.contains(kind)).unwrap_or("hierarchy");
    let title = Some(field(&plan, "title")).filter(|title| !title.is_empty()).unwrap_or_else(|| section_title.trim().to_string());
    let data = json!({ "nodes": nodes, "edges": edges, "dropped": dropped });
    conn.execute(
        "INSERT INTO diagrams (book_id, section_id, kind, title, data, model) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![book_id, section_id, kind, cut(&title, 120), data.to_string(), model],
    )?;
    Ok(conn.query_row("SELECT * FROM diagrams WHERE id = ?", [conn.last_insert_rowid()], diagram_view)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{analyse, Scope};
    use crate::llm::testing::FakeLlm;
    use crate::testing::{book, library, section_id};

    const TREATING: &str = "Water treatment follows a fixed order. 1. Screening removes leaves and large debris. \
        2. Coagulation makes fine particles clump together. 3. Sedimentation lets the clumps settle. \
        4. Filtration passes the water through sand. Each stage depends on the one before it.";

    fn water(conn: &Connection) -> (i64, i64) {
        let id = book(conn, "Water and Air", &[
            ("Treating water", &[TREATING, "The process by which fine particles clump together is called coagulation."]),
            ("Empty", &[]),
        ]);
        (id, section_id(conn, "Treating water"))
    }

    fn plan() -> Value {
        json!({
            "kind": "process", "title": "How water is treated",
            "nodes": [
                { "id": "n1", "label": "Screening", "quote": "Screening removes leaves and large debris" },
                { "id": "n2", "label": "Coagulation", "quote": "Coagulation makes fine particles clump together" },
                { "id": "n3", "label": "Chlorination", "quote": "Chlorine is added to kill every remaining germ" },
            ],
            "edges": [{ "from": "n1", "to": "n2", "label": "then" }, { "from": "n2", "to": "n3", "label": "then" }],
        })
    }

    fn saved(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM diagrams", [], |row| row.get(0)).unwrap()
    }

    #[test]
    fn a_model_diagram_keeps_only_what_the_text_supports() {
        let conn = library();
        let (book_id, section) = water(&conn);
        let fake = FakeLlm::replying(plan());
        let diagram = draw_section(&conn, &fake, section, "local:1", &json!({})).unwrap();
        let labels: Vec<&str> = diagram["nodes"].as_array().unwrap().iter().map(|node| node["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Screening", "Coagulation"]); // the invented step is gone
        assert_eq!((&diagram["edges"], &diagram["dropped"]), (&json!([{ "source": "n1", "target": "n2", "label": "then" }]), &json!(1)));
        assert_eq!((&diagram["kind"], &diagram["model"], &diagram["title"], &diagram["section_id"]), (&json!("process"), &json!("local:1"), &json!("How water is treated"), &json!(section)));
        assert_eq!((&diagram["nodes"][0]["start_page"], &diagram["nodes"][0]["section_title"]), (&json!(2), &json!("Treating water")));
        assert!(fake.prompt(0).starts_with("Section: Treating water\n\nText:\nWater treatment follows") && fake.prompt(0).contains("Screening removes leaves"));
        // It is kept with the book, where the Visualise screen lists it for its section.
        let listed = analyse(&conn, book_id, Scope { section_id: Some(section), chapter_id: None }).unwrap().diagrams;
        assert_eq!(listed, [diagram]);
    }

    #[test]
    fn what_the_model_leaves_vague_is_filled_in_or_left_out() {
        let conn = library();
        let (_, section) = water(&conn);
        let fake = FakeLlm::replying(json!({
            "kind": "flowchart", "title": "  ",
            "nodes": [
                { "id": "a", "label": "L".repeat(200), "quote": "Screening removes leaves and large debris" },
                { "id": "a", "label": "Repeated id", "quote": "Coagulation makes fine particles clump together" },
                { "id": "b", "label": "", "quote": "Sedimentation lets the clumps settle" },
                { "id": "c", "label": "Filtration", "quote": "Filtration passes the water through sand" },
            ],
            "edges": [{ "from": "a", "to": "a" }, { "from": "a", "to": "zz" }, { "from": "c", "to": "a" }, "nonsense"],
        }));
        let diagram = draw_section(&conn, &fake, section, "local:1", &json!({})).unwrap();
        assert_eq!((&diagram["kind"], &diagram["title"], &diagram["dropped"]), (&json!("hierarchy"), &json!("Treating water"), &json!(2)));
        assert_eq!(diagram["nodes"][0]["label"].as_str().unwrap().chars().count(), 80);
        assert_eq!(diagram["edges"], json!([{ "source": "c", "target": "a", "label": "" }]));
    }

    #[test]
    fn a_diagram_with_nothing_supported_is_refused_and_not_saved() {
        let conn = library();
        let (_, section) = water(&conn);
        let mut invented = plan();
        invented["nodes"] = json!([{ "id": "n1", "label": "Magic", "quote": "the water is purified by magic alone" }]);
        let refused = draw_section(&conn, &FakeLlm::replying(invented), section, "local:1", &json!({}));
        assert!(refused.unwrap_err().to_string().contains("could not be matched"));
        assert_eq!(saved(&conn), 0);
    }

    #[test]
    fn problems_with_the_model_or_the_section_are_reported() {
        let conn = library();
        let (_, section) = water(&conn);
        for reply in [json!("not json"), json!({ "kind": "process" })] {
            let refused = draw_section(&conn, &FakeLlm::replying(reply), section, "local:1", &json!({}));
            assert_eq!(refused.unwrap_err().to_string(), "The model returned a diagram that could not be read. Please try again.");
        }
        let fake = FakeLlm::new();
        assert_eq!(draw_section(&conn, &fake, section_id(&conn, "Empty"), "local:1", &json!({})).unwrap_err().to_string(), "This section has no text to draw from.");
        assert!(matches!(draw_section(&conn, &fake, 99_999, "local:1", &json!({})), Err(Error::NotFound("Section"))));
        assert_eq!(fake.call_count(), 0);
        fake.go_down();
        assert!(matches!(draw_section(&conn, &fake, section, "local:1", &json!({})), Err(Error::Model(ModelProblem::Unavailable, _))));
        assert_eq!(saved(&conn), 0);
    }
}
