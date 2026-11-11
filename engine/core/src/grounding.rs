//! Checking what the model says the book says against what the book says.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::retrieve::Retrieved;

const MIN_QUOTE_WORDS: usize = 3;

/// Text reduced to what is compared: lower case, plain quotation marks, single spaces, no punctuation.
pub fn normalize_for_match(text: &str) -> String {
    static GAP: OnceLock<Regex> = OnceLock::new();
    let plain = text.replace(['’', '‘'], "'").replace(['“', '”'], "\"").to_lowercase();
    GAP.get_or_init(|| Regex::new(r#"[^\w'"]+"#).unwrap()).replace_all(&plain, " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the quoted words, at least three of them, stand together in the passage.
pub fn quote_appears_in(quote: &str, passage: &str) -> bool {
    let normalized = normalize_for_match(quote);
    normalized.split(' ').filter(|word| !word.is_empty()).count() >= MIN_QUOTE_WORDS && normalize_for_match(passage).contains(&normalized)
}

/// The text of a field the model was asked for, whatever it actually sent there.
pub fn field(item: &Value, name: &str) -> String {
    match &item[name] {
        Value::String(text) => text.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string().trim().to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Citation {
    pub passage_id: i64,
    pub section_id: i64,
    pub chapter_title: String,
    pub section_title: String,
    pub start_page: i64,
    pub end_page: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Segment {
    /// supported, background, interpretation or not_found.
    pub kind: String,
    pub text: String,
    pub quote: Option<String>,
    pub citation: Option<Citation>,
    /// The model claimed the book's support, and the claim did not hold.
    pub downgraded: bool,
}

/// Checks every claim of the book's support against the passage it names.
///
/// A "supported" segment whose quotation cannot be found is downgraded, so the model can never
/// present invented words as coming from the book.
pub fn verify_segments(raw: &[Value], labelled: &[(String, Retrieved)]) -> Vec<Segment> {
    let mut segments = Vec::new();
    for item in raw {
        let kind = item["kind"].as_str().unwrap_or("");
        let text = field(item, "text");
        if text.is_empty() || !["supported", "background", "interpretation", "not_found"].contains(&kind) {
            continue;
        }
        let mut segment = Segment { kind: kind.to_string(), text, quote: None, citation: None, downgraded: false };
        if kind == "supported" {
            let source = field(item, "source");
            let source = source.trim_matches(|c| c == '[' || c == ']');
            let quote = field(item, "quote");
            match labelled.iter().find(|(label, _)| label == source) {
                Some((_, passage)) if quote_appears_in(&quote, &passage.text) => {
                    segment.quote = Some(quote);
                    segment.citation = Some(Citation {
                        passage_id: passage.id,
                        section_id: passage.section_id,
                        chapter_title: passage.chapter_title.clone(),
                        section_title: passage.section_title.clone(),
                        start_page: passage.start_page,
                        end_page: passage.end_page,
                    });
                }
                _ => {
                    segment.kind = "interpretation".into();
                    segment.downgraded = true;
                }
            }
        }
        segments.push(segment);
    }
    segments
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TEXT: &str = "Mitochondria produce most of the chemical energy that a cell needs.";

    fn passage() -> Vec<(String, Retrieved)> {
        vec![("P1".into(), Retrieved {
            id: 9, text: TEXT.into(), chapter_id: 1, section_id: 2, chapter_title: "C".into(), section_title: "S".into(), start_page: 41, end_page: 42, score: 0.0,
        })]
    }

    #[test]
    fn a_quotation_must_be_the_books_words_and_long_enough() {
        for (quote, expected) in [
            ("produce most of the chemical energy", true),
            ("PRODUCE most  of the chemical, energy", true),
            ("produce all of the chemical energy", false),
            ("produce most", false),
            ("", false),
            ("... , ;", false),
        ] {
            assert_eq!(quote_appears_in(quote, TEXT), expected, "{quote:?}");
        }
        assert!(quote_appears_in("the cell’s “power house”", "It is the cell's \"power house\", they say."));
    }

    #[test]
    fn a_real_quotation_is_cited_with_the_stored_pages() {
        let raw = [json!({ "kind": "supported", "text": " They make energy. ", "source": "[P1]", "quote": "most of the chemical energy" })];
        let segment = &verify_segments(&raw, &passage())[0];
        assert_eq!((segment.kind.as_str(), segment.text.as_str(), segment.downgraded), ("supported", "They make energy.", false));
        let citation = segment.citation.as_ref().unwrap();
        assert_eq!((citation.passage_id, citation.section_id, citation.start_page, citation.end_page), (9, 2, 41, 42));
        assert_eq!(segment.quote.as_deref(), Some("most of the chemical energy"));
    }

    #[test]
    fn support_that_cannot_be_checked_is_downgraded() {
        for raw in [
            json!({ "kind": "supported", "text": "x", "source": "P1", "quote": "an invented sentence never in the book" }),
            json!({ "kind": "supported", "text": "x", "source": "P7", "quote": "most of the chemical energy" }),
            json!({ "kind": "supported", "text": "x", "source": null, "quote": null }),
            json!({ "kind": "supported", "text": "x", "source": 1, "quote": ["most of the chemical energy"] }),
        ] {
            let segment = &verify_segments(&[raw], &passage())[0];
            assert_eq!((segment.kind.as_str(), segment.downgraded, &segment.citation, &segment.quote), ("interpretation", true, &None, &None));
        }
    }

    #[test]
    fn segments_without_text_or_of_an_unknown_kind_are_left_out() {
        let raw = [
            json!({ "kind": "background", "text": "Kept." }),
            json!({ "kind": "background", "text": "   " }),
            json!({ "kind": "rumour", "text": "Dropped." }),
            json!("not even an object"),
            json!({ "kind": "not_found", "text": "The document does not say.", "source": "P1", "quote": "most of the chemical energy" }),
        ];
        let segments = verify_segments(&raw, &passage());
        assert_eq!(segments.iter().map(|segment| segment.kind.as_str()).collect::<Vec<_>>(), ["background", "not_found"]);
        assert!(segments.iter().all(|segment| segment.citation.is_none() && !segment.downgraded));
    }
}
