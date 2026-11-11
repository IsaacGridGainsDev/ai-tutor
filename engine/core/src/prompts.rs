//! Every prompt the application sends to a model lives here, with the shape its reply must have.

use serde_json::{json, Value};

pub const GROUNDED_ANSWER_SYSTEM: &str = r#"You are a careful tutor answering a learner's question about a document.
You are given numbered passages from that document. Break your answer into segments, each with a kind:
- "supported": stated directly by a passage. Set "source" to its label (for example "P2") and "quote" to words copied exactly from that passage, 5 to 25 words long.
- "background": correct general knowledge that the passages do not state. "source" and "quote" are null.
- "interpretation": your own inference or explanation connecting ideas. "source" and "quote" are null.
- "not_found": the passages do not contain this information. Say so plainly. "source" and "quote" are null.
Rules:
- Never present anything as "supported" unless the quoted words really appear in that passage.
- If the passages do not answer the question, begin with a "not_found" segment. Do not guess.
- Use plain language and be concise."#;

pub fn grounded_answer_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "segments": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["supported", "background", "interpretation", "not_found"] },
                        "text": { "type": "string" },
                        "source": { "type": ["string", "null"] },
                        "quote": { "type": ["string", "null"] },
                    },
                    "required": ["kind", "text", "source", "quote"],
                },
            }
        },
        "required": ["segments"],
    })
}

pub const CONCEPT_EXTRACTION_SYSTEM: &str = r#"You identify the key concepts a learner must understand from a chapter excerpt.
Return at most 12 concepts. For each give:
- "name": the concept, 1 to 5 words, in lower case unless it is a proper noun.
- "definition": one sentence, based only on the excerpt.
- "quote": words copied exactly from the excerpt that support it, 5 to 25 words long.
Only include concepts the excerpt actually explains. Do not include people, examples or section labels."#;

pub fn concept_extraction_user(title: &str, excerpt: &str) -> String {
    format!("Chapter: {title}\n\nExcerpt:\n{excerpt}")
}

pub fn concept_extraction_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "concepts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "name": { "type": "string" }, "definition": { "type": "string" }, "quote": { "type": "string" } },
                    "required": ["name", "definition", "quote"],
                },
            }
        },
        "required": ["concepts"],
    })
}

pub const DIAGRAM_SYSTEM: &str = r#"You plan a diagram that explains a section of a textbook to a learner.
Choose the kind that fits what the section says:
- "process": steps that happen in order.
- "hierarchy": a whole and its parts, or a category and its members.
- "comparison": two or more things set against each other.
- "cause_effect": causes leading to effects.
Return at most 10 nodes. For each give:
- "id": a short unique identifier such as "n1".
- "label": 1 to 6 words naming the step, part, thing or cause.
- "quote": words copied exactly from the section that state it, 5 to 25 words long.
Return edges as {"from": id, "to": id, "label": a few words or an empty string}. For a process, link each step to the next.
Use only what the section says. Do not add steps, parts or causes from your own knowledge."#;

pub fn diagram_user(title: &str, excerpt: &str) -> String {
    format!("Section: {title}\n\nText:\n{excerpt}")
}

pub fn diagram_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": ["process", "hierarchy", "comparison", "cause_effect"] },
            "title": { "type": "string" },
            "nodes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "id": { "type": "string" }, "label": { "type": "string" }, "quote": { "type": "string" } },
                    "required": ["id", "label", "quote"],
                },
            },
            "edges": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "from": { "type": "string" }, "to": { "type": "string" }, "label": { "type": "string" } },
                    "required": ["from", "to"],
                },
            },
        },
        "required": ["kind", "title", "nodes", "edges"],
    })
}
