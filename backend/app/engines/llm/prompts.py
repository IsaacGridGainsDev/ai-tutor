"""Every prompt the application sends to a model lives here, versioned by name."""
from typing import Any

GROUNDED_ANSWER_SYSTEM = """You are a careful tutor answering a learner's question about a document.
You are given numbered passages from that document. Break your answer into segments, each with a kind:
- "supported": stated directly by a passage. Set "source" to its label (for example "P2") and "quote" to words copied exactly from that passage, 5 to 25 words long.
- "background": correct general knowledge that the passages do not state. "source" and "quote" are null.
- "interpretation": your own inference or explanation connecting ideas. "source" and "quote" are null.
- "not_found": the passages do not contain this information. Say so plainly. "source" and "quote" are null.
Rules:
- Never present anything as "supported" unless the quoted words really appear in that passage.
- If the passages do not answer the question, begin with a "not_found" segment. Do not guess.
- Use plain language and be concise."""

GROUNDED_ANSWER_USER = """Passages:
{passages}
{concepts}
Question: {question}"""

GROUNDED_ANSWER_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "segments": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "kind": {"type": "string", "enum": ["supported", "background", "interpretation", "not_found"]},
                    "text": {"type": "string"},
                    "source": {"type": ["string", "null"]},
                    "quote": {"type": ["string", "null"]},
                },
                "required": ["kind", "text", "source", "quote"],
            },
        }
    },
    "required": ["segments"],
}

CONCEPT_EXTRACTION_SYSTEM = """You identify the key concepts a learner must understand from a chapter excerpt.
Return at most 12 concepts. For each give:
- "name": the concept, 1 to 5 words, in lower case unless it is a proper noun.
- "definition": one sentence, based only on the excerpt.
- "quote": words copied exactly from the excerpt that support it, 5 to 25 words long.
Only include concepts the excerpt actually explains. Do not include people, examples or section labels."""

CONCEPT_EXTRACTION_USER = """Chapter: {title}

Excerpt:
{excerpt}"""

CONCEPT_EXTRACTION_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "concepts": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "definition": {"type": "string"},
                    "quote": {"type": "string"},
                },
                "required": ["name", "definition", "quote"],
            },
        }
    },
    "required": ["concepts"],
}
