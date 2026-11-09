import re
from typing import Any

MIN_QUOTE_WORDS = 3


def normalize_for_match(text: str) -> str:
    text = text.replace("’", "'").replace("‘", "'").replace("“", '"').replace("”", '"')
    return " ".join(re.sub(r"[^\w'\"]+", " ", text.lower()).split())


def quote_appears_in(quote: str, passage: str) -> bool:
    normalized = normalize_for_match(quote)
    return len(normalized.split()) >= MIN_QUOTE_WORDS and normalized in normalize_for_match(passage)


def verify_segments(raw: list[dict[str, Any]], passages_by_label: dict[str, Any]) -> list[dict[str, Any]]:
    """Check every claim of textbook support against the actual passage text.

    A "supported" segment whose quote cannot be found is downgraded, so the model
    can never present an invented quotation as coming from the book.
    """
    segments = []
    for item in raw:
        kind = item.get("kind")
        text = str(item.get("text") or "").strip()
        if not text or kind not in ("supported", "background", "interpretation", "not_found"):
            continue
        segment: dict[str, Any] = {"kind": kind, "text": text, "quote": None, "citation": None, "downgraded": False}
        if kind == "supported":
            passage = passages_by_label.get(str(item.get("source") or "").strip().strip("[]"))
            quote = str(item.get("quote") or "")
            if passage is not None and quote_appears_in(quote, passage.text):
                segment["quote"] = quote
                segment["citation"] = {
                    "passage_id": passage.id,
                    "section_id": passage.section_id,
                    "chapter_title": passage.chapter_title,
                    "section_title": passage.section_title,
                    "start_page": passage.start_page,
                    "end_page": passage.end_page,
                }
            else:
                segment["kind"] = "interpretation"
                segment["downgraded"] = True
        segments.append(segment)
    return segments
