//! Answering a question about a book from the book's own passages.

use std::time::Instant;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::embeddings::Embedder;
use crate::grounding::verify_segments;
use crate::llm::{Flow, Llm, Message};
use crate::retrieve::{identify_subject, related_concepts, retrieve, Concept, Retrieved};
use crate::vectors::VectorStore;
use crate::{ai, prompts, settings, Error, ModelProblem, Result};

const MAX_CONTEXT_CHARS: usize = 5000;
const MAX_PASSAGES: usize = 4;
const RETRIEVED: usize = 5;
const MIN_QUESTION_CHARS: usize = 3;
const MAX_QUESTION_CHARS: usize = 2000;
const NOT_IN_DOCUMENT: &str = "I could not find anything about this in the document.";

/// Labels passages for the model, keeping the prompt small enough for a model on the CPU.
fn context(passages: Vec<Retrieved>) -> (String, Vec<(String, Retrieved)>) {
    let mut parts = Vec::new();
    let mut labelled = Vec::new();
    let mut used = 0;
    for (index, passage) in passages.into_iter().take(MAX_PASSAGES).enumerate() {
        let length = passage.text.chars().count();
        if used > 0 && used + length > MAX_CONTEXT_CHARS {
            break;
        }
        let label = format!("P{}", index + 1);
        used += length;
        parts.push(format!("[{label}] ({}, page {})\n{}", passage.section_title, passage.start_page, passage.text));
        labelled.push((label, passage));
    }
    (parts.join("\n\n"), labelled)
}

fn concept_notes(concepts: &[Concept]) -> String {
    let lines: Vec<String> = concepts.iter().filter_map(|concept| {
        concept.definition.as_deref().filter(|definition| !definition.is_empty()).map(|definition| format!("- {}: {definition}", concept.name))
    }).collect();
    if lines.is_empty() { String::new() } else { format!("Related concepts from this document:\n{}\n", lines.join("\n")) }
}

fn unreadable(message: &str) -> Error {
    Error::Model(ModelProblem::Failed, message.into())
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Runs the knowledge pipeline, emitting progress events and finally a checked answer.
/// When the listener asks to stop, generation ends and nothing more is emitted.
#[allow(clippy::too_many_arguments)]
pub fn answer_question(
    conn: &Connection, llm: &dyn Llm, vectors: &VectorStore, embedder: &Embedder, book_id: i64, question: &str, model: &str,
    options: &Value, section_id: Option<i64>, emit: &mut dyn FnMut(Value) -> Flow,
) -> Result<()> {
    if emit(json!({ "type": "stage", "stage": "retrieving" })) == Flow::Stop {
        return Ok(());
    }
    let subject = identify_subject(conn, book_id, question)?;
    let passages = retrieve(conn, book_id, question, &subject, vectors, embedder, RETRIEVED, section_id)?;
    if passages.is_empty() {
        // Nothing in the book shares a word with the question, so there is nothing to ground on,
        // and the model is not asked: it would answer from somewhere else.
        emit(json!({
            "type": "answer",
            "found_in_document": false,
            "segments": [{ "kind": "not_found", "text": NOT_IN_DOCUMENT, "quote": null, "citation": null, "downgraded": false }],
            "passages": [],
            "concepts": [],
            "model": null,
        }));
        return Ok(());
    }

    let (context, labelled) = context(passages);
    let used: Vec<&Retrieved> = labelled.iter().map(|(_, passage)| passage).collect();
    let concepts = related_concepts(conn, &subject, &used, 6)?;
    let messages = [
        Message::system(prompts::GROUNDED_ANSWER_SYSTEM),
        Message::user(format!("Passages:\n{context}\n{}\nQuestion: {question}", concept_notes(&concepts))),
    ];

    if emit(json!({ "type": "stage", "stage": "generating" })) == Flow::Stop {
        return Ok(());
    }
    let started = Instant::now();
    let mut content = String::new();
    let (mut tokens, mut prompt_tokens, mut completion_tokens, mut generation_seconds) = (0, 0, 0, 0.0);
    let mut stopped = false;
    llm.chat(model, &messages, Some(&prompts::grounded_answer_schema()), options, &mut |chunk| {
        content.push_str(&chunk.delta);
        if chunk.done {
            (prompt_tokens, completion_tokens, generation_seconds) = (chunk.prompt_tokens, chunk.completion_tokens, chunk.generation_seconds);
        }
        if !chunk.delta.is_empty() {
            tokens += 1;
            // Frequent events also let a learner who pressed Stop be noticed quickly.
            if tokens % 5 == 0 && emit(json!({ "type": "progress", "tokens": tokens })) == Flow::Stop {
                stopped = true;
                return Flow::Stop;
            }
        }
        Flow::Continue
    })?;
    if stopped {
        return Ok(());
    }

    let reply: Value = serde_json::from_str(&content).map_err(|_| unreadable("The model returned an answer that could not be read. Please try again."))?;
    let raw = reply["segments"].as_array().ok_or_else(|| unreadable("The model returned an answer that could not be read. Please try again."))?;
    let segments = verify_segments(raw, &labelled);
    if segments.is_empty() {
        return Err(unreadable("The model returned an empty answer. Please try again."));
    }

    let passages: Vec<Value> = labelled.iter().map(|(label, passage)| {
        let mut view = serde_json::to_value(passage).expect("a passage is plain data");
        view["label"] = json!(label);
        view
    }).collect();
    emit(json!({
        "type": "answer",
        "found_in_document": segments.iter().any(|segment| segment.kind == "supported"),
        "segments": segments,
        "passages": passages,
        "concepts": concepts.iter().map(|concept| concept.name.as_str()).collect::<Vec<_>>(),
        "model": model,
        "stats": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "generation_seconds": round2(generation_seconds),
            "total_seconds": round2(started.elapsed().as_secs_f64()),
        },
    }));
    Ok(())
}

/// Answers a question about a book with the models chosen in Settings.
///
/// A question that cannot be asked at all is refused. Once asking has begun, a failure of the
/// model arrives as a final "error" event, as progress does, instead of as a refusal.
pub fn ask(
    conn: &Connection, llm: &dyn Llm, vectors: &VectorStore, book_id: i64, question: &str, section_id: Option<i64>,
    emit: &mut dyn FnMut(Value) -> Flow,
) -> Result<()> {
    let question = question.trim();
    let length = question.chars().count();
    if !(MIN_QUESTION_CHARS..=MAX_QUESTION_CHARS).contains(&length) {
        return Err(Error::Invalid(format!("A question must be between {MIN_QUESTION_CHARS} and {MAX_QUESTION_CHARS} characters.")));
    }
    let status: Option<String> = conn.query_row("SELECT status FROM books WHERE id = ?", [book_id], |row| row.get(0)).optional()?;
    match status.as_deref() {
        None => return Err(Error::NotFound("Book")),
        Some("ready") => {}
        Some(_) => return Err(Error::Invalid("This document is still being processed.".into())),
    }

    let answered = (|| {
        let settings = settings::load(conn)?;
        let models = llm.list_models()?;
        let embedder = ai::resolve_embedder(llm, &models, settings.embedding_model.as_deref());
        let model = ai::resolve_chat_model(&models, settings.llm_model.as_deref())?;
        answer_question(conn, llm, vectors, &embedder, book_id, question, &model, &ai::inference_options(&settings), section_id, emit)
    })();
    match answered {
        Err(Error::Model(_, message)) => {
            emit(json!({ "type": "error", "message": message }));
            Ok(())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::llm::testing::{model, FakeLlm};
    use crate::testing::{chaptered_book, knowledge_library, scratch_folder, KnowledgeLibrary};

    const QUESTION: &str = "What do mitochondria produce?";

    fn asked(conn: &Connection, fake: &FakeLlm, book: i64, question: &str, folder: &str) -> Vec<Value> {
        let directory = scratch_folder(folder);
        let mut events = Vec::new();
        ask(conn, fake, &VectorStore::new(directory.clone()), book, question, None, &mut |event| {
            events.push(event);
            Flow::Continue
        }).unwrap();
        let _ = std::fs::remove_dir_all(directory);
        events
    }

    #[test]
    fn an_answer_is_grounded_and_cites_real_pages() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::replying(json!({ "segments": [
            { "kind": "supported", "text": "Mitochondria make the energy.", "source": "P1", "quote": "Mitochondria produce most of the chemical energy" },
            { "kind": "background", "text": "This happens in nearly all animal cells.", "source": null, "quote": null },
        ] }));
        let events = asked(&conn, &fake, knowledge, QUESTION, "answer-grounded");
        let stages: Vec<&str> = events.iter().filter(|event| event["type"] == "stage").map(|event| event["stage"].as_str().unwrap()).collect();
        assert_eq!(stages, ["retrieving", "generating"]);
        assert!(events.iter().any(|event| event["type"] == "progress" && event["tokens"] == 5));
        let answer = events.last().unwrap();
        assert_eq!((&answer["type"], &answer["found_in_document"], &answer["model"]), (&json!("answer"), &json!(true), &json!("local:1")));
        assert_eq!(answer["segments"][0]["citation"]["start_page"], 2);
        assert_eq!(answer["segments"][0]["citation"]["section_title"], "Energy in cells");
        assert_eq!(answer["segments"][1]["kind"], "background");
        assert_eq!((&answer["stats"]["completion_tokens"], &answer["stats"]["prompt_tokens"], &answer["stats"]["generation_seconds"]), (&json!(22), &json!(11), &json!(1.5)));
        assert_eq!(answer["passages"][0]["label"], "P1");
        // What the model was sent: the passage, labelled and located, and the question.
        let prompt = fake.prompt(0);
        assert!(prompt.starts_with("Passages:\n[P1] (Energy in cells, page 2)\nMitochondria produce most") && prompt.ends_with(&format!("\nQuestion: {QUESTION}")));
        let calls = fake.calls.lock().unwrap();
        assert_eq!((calls[0].model.as_str(), &calls[0].options["num_ctx"]), ("local:1", &json!(8192)));
    }

    #[test]
    fn the_prompt_holds_only_the_relevant_passages_not_the_book() {
        let conn = crate::testing::library();
        let paragraph = "Energy and motion are related in ways that this long paragraph keeps describing at length. ".repeat(12);
        let passages: Vec<&str> = vec![paragraph.as_str(); 60];
        let book = chaptered_book(&conn, "Long", &[("Chapter 1", &[("1.1 Motion", passages.as_slice())])]);
        let fake = FakeLlm::new();
        asked(&conn, &fake, book, "How does energy relate to motion?", "answer-long");
        let prompt = fake.prompt(0).chars().count();
        assert!(prompt < 7000 && 7000 < paragraph.chars().count() * 60, "{prompt}");
    }

    #[test]
    fn an_invented_quotation_is_not_presented_as_the_book() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::replying(json!({ "segments": [{ "kind": "supported", "text": "Claim.", "source": "P1", "quote": "mitochondria were discovered in Paris during winter" }] }));
        let events = asked(&conn, &fake, knowledge, QUESTION, "answer-invented");
        let answer = events.last().unwrap();
        assert_eq!(answer["found_in_document"], false);
        assert_eq!((&answer["segments"][0]["kind"], &answer["segments"][0]["citation"], &answer["segments"][0]["downgraded"]), (&json!("interpretation"), &Value::Null, &json!(true)));
    }

    #[test]
    fn a_question_unrelated_to_the_book_never_reaches_the_model() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::new();
        let events = asked(&conn, &fake, knowledge, "Which volcano erupted?", "answer-unrelated");
        let answer = events.last().unwrap();
        assert_eq!((&answer["found_in_document"], &answer["segments"][0]["kind"], &answer["model"]), (&json!(false), &json!("not_found"), &Value::Null));
        assert_eq!(fake.call_count(), 0);
    }

    #[test]
    fn the_model_can_say_the_book_does_not_say() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::replying(json!({ "segments": [{ "kind": "not_found", "text": "The document does not say.", "source": null, "quote": null }] }));
        let events = asked(&conn, &fake, knowledge, "What energy did Einstein describe?", "answer-not-found");
        let answer = events.last().unwrap();
        assert_eq!((&answer["found_in_document"], &answer["segments"][0]["kind"]), (&json!(false), &json!("not_found")));
    }

    #[test]
    fn a_reply_that_cannot_be_read_becomes_an_error_event() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        for reply in [json!("not json at all"), json!({ "wrong": 1 }), json!({ "segments": [] }), json!({ "segments": "many" })] {
            let events = asked(&conn, &FakeLlm::replying(reply.clone()), knowledge, QUESTION, "answer-unreadable");
            let last = events.last().unwrap();
            assert_eq!(last["type"], "error", "{reply}");
            assert!(last["message"].as_str().unwrap().ends_with("Please try again."));
        }
    }

    #[test]
    fn a_runtime_that_is_down_becomes_an_error_event() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::new();
        fake.go_down();
        let events = asked(&conn, &fake, knowledge, QUESTION, "answer-down");
        assert_eq!(events, [json!({ "type": "error", "message": "Ollama is not running. Start Ollama and try again." })]);
    }

    #[test]
    fn only_a_cloud_model_installed_means_no_answer_and_nothing_sent() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::new().with_models(vec![model("remote:1", &["completion"], true)]);
        let events = asked(&conn, &fake, knowledge, QUESTION, "answer-cloud");
        assert_eq!(events.last().unwrap()["type"], "error");
        assert_eq!(fake.call_count(), 0);
    }

    #[test]
    fn a_question_that_cannot_be_asked_is_refused() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::new();
        let store = VectorStore::new(scratch_folder("answer-refused"));
        let try_ask = |book: i64, question: &str| ask(&conn, &fake, &store, book, question, None, &mut |_| Flow::Continue);
        assert!(matches!(try_ask(999, "anything?"), Err(Error::NotFound("Book"))));
        assert!(matches!(try_ask(knowledge, " a "), Err(Error::Invalid(_))));
        assert!(matches!(try_ask(knowledge, &"q".repeat(2001)), Err(Error::Invalid(_))));
        conn.execute("UPDATE books SET status = 'indexing' WHERE id = ?", [knowledge]).unwrap();
        assert_eq!(try_ask(knowledge, "anything?").unwrap_err().to_string(), "This document is still being processed.");
        assert_eq!(fake.call_count(), 0);
    }

    #[test]
    fn stopping_ends_generation_and_no_answer_follows() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let fake = FakeLlm::replying(json!({ "segments": [{ "kind": "background", "text": "x".repeat(4000), "source": null, "quote": null }] }));
        let directory = scratch_folder("answer-stopped");
        let mut events = Vec::new();
        ask(&conn, &fake, &VectorStore::new(directory.clone()), knowledge, QUESTION, None, &mut |event| {
            let stop = event["type"] == "progress";
            events.push(event);
            if stop { Flow::Stop } else { Flow::Continue }
        }).unwrap();
        let _ = std::fs::remove_dir_all(directory);
        assert_eq!(fake.stopped.load(Ordering::SeqCst), 1);
        assert_eq!(events.last().unwrap(), &json!({ "type": "progress", "tokens": 5 }));
        assert!(events.iter().all(|event| event["type"] != "answer"));
    }
}
