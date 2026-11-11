//! Turning text into vectors, so passages can be ranked by likeness to a question.

use std::sync::OnceLock;

use regex::Regex;

use crate::llm::Llm;
use crate::{Error, ModelProblem, Result};

const STOPWORDS: &str = "a an and are as at be been but by can could did do does for from had has have how i if in into is it its \
    me my of on or our should so than that the their them then there these they this to us was we were what when \
    where which who why will with would you your about explain tell please mean means meant";

fn is_stopword(word: &str) -> bool {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| STOPWORDS.split_whitespace().collect()).contains(&word)
}

/// The words of a text that say what it is about, in lower case: runs of letters, or of digits.
pub fn content_words(text: &str) -> Vec<String> {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let word = WORD.get_or_init(|| Regex::new(r"[\w--[\d_]]+|\d+").unwrap());
    word.find_iter(text).map(|found| found.as_str().to_lowercase()).filter(|found| !is_stopword(found)).collect()
}

/// A deliberately small stemmer: enough to unify plurals and common verb endings.
pub fn stem(word: &str) -> String {
    let length = word.chars().count();
    for suffix in ["ing", "ies", "ed", "es", "s"] {
        if word.ends_with(suffix) && length - suffix.len() >= 3 {
            let root = &word[..word.len() - suffix.len()];
            return if suffix == "ies" { format!("{root}y") } else { root.to_string() };
        }
    }
    word.to_string()
}

pub const HASHING_DIMENSIONS: usize = 4096;
const MODEL_BATCH: usize = 32;

fn hashed_features(text: &str) -> Vec<(String, f32)> {
    let stems: Vec<String> = content_words(text).iter().map(|word| stem(word)).collect();
    let mut features: Vec<(String, f32)> = stems.iter().map(|word| (word.clone(), 1.0)).collect();
    features.extend(stems.windows(2).map(|pair| (format!("{}_{}", pair[0], pair[1]), 1.5)));
    for word in &stems {
        let padded: Vec<char> = format!("<{word}>").chars().collect();
        features.extend(padded.windows(4).map(|gram| (gram.iter().collect(), 0.2)));
    }
    features
}

fn normalise(vector: &mut [f32]) {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        vector.iter_mut().for_each(|value| *value /= length);
    }
}

/// Vectors from hashed words, word pairs and letter groups, needing no model.
///
/// This captures spelling and word-form likeness, not meaning. It is what is used until an
/// embedding model is installed and chosen.
fn hashing_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; HASHING_DIMENSIONS];
    for (feature, weight) in hashed_features(text) {
        let digest = crc32fast::hash(feature.as_bytes());
        let sign = if digest & 1 == 1 { 1.0 } else { -1.0 };
        vector[(digest >> 1) as usize % HASHING_DIMENSIONS] += sign * weight;
    }
    // Dampen repeated terms so long passages are not dominated by a few words.
    vector.iter_mut().for_each(|value| *value = value.signum() * value.abs().sqrt());
    normalise(&mut vector);
    vector
}

pub enum Embedder<'a> {
    Hashing,
    /// Vectors that carry meaning, from a local embedding model.
    Model { llm: &'a dyn Llm, model: String },
}

impl Embedder<'_> {
    /// Names the kind of vector, so an index made with one embedder is never searched with another.
    pub fn key(&self) -> String {
        match self {
            Self::Hashing => "hashing-v1".into(),
            Self::Model { model, .. } => {
                let safe: String = model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
                format!("model-{safe}")
            }
        }
    }

    /// One vector of length 1 for each text.
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self {
            Self::Hashing => Ok(texts.iter().map(|text| hashing_vector(text)).collect()),
            Self::Model { llm, model } => {
                let mut vectors = Vec::with_capacity(texts.len());
                for batch in texts.chunks(MODEL_BATCH) {
                    vectors.extend(llm.embed(model, batch)?);
                }
                let width = vectors.first().map_or(0, Vec::len);
                if vectors.len() != texts.len() || width == 0 || vectors.iter().any(|vector| vector.len() != width) {
                    return Err(Error::Model(ModelProblem::Failed, format!("The embedding model {model} returned vectors that could not be used.")));
                }
                vectors.iter_mut().for_each(|vector| normalise(vector));
                Ok(vectors)
            }
        }
    }
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::testing::FakeLlm;

    fn texts(all: &[&str]) -> Vec<String> {
        all.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn content_words_drop_filler_and_split_letters_from_digits() {
        assert_eq!(content_words("What do the Mitochondria produce, please?"), ["mitochondria", "produce"]);
        assert_eq!(content_words("CO2 rose_by 12% in naïve tests"), ["co", "2", "rose", "12", "naïve", "tests"]);
        assert!(content_words("Why is it so?").is_empty());
    }

    #[test]
    fn stems_unify_common_endings_but_keep_short_words() {
        assert_eq!(["functions", "calling", "bodies", "pushed", "boxes", "gas", "is"].map(stem), ["function", "call", "body", "push", "box", "gas", "is"]);
    }

    #[test]
    fn hashing_vectors_have_length_one_and_tolerate_word_forms() {
        let vectors = Embedder::Hashing.embed(&texts(&["recursive functions call themselves", "a recursive function", "the weather in spring"])).unwrap();
        for vector in &vectors {
            assert_eq!(vector.len(), HASHING_DIMENSIONS);
            assert!((dot(vector, vector) - 1.0).abs() < 1e-5);
        }
        assert!(dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[2]));
    }

    #[test]
    fn hashing_matches_the_vectors_the_python_backend_made() {
        // Values printed by the Python embedder before it was removed. The checksum of "cell", as
        // zlib computes it, fixes where the word lands (place 1009) and its sign (even: negative).
        assert_eq!(crc32fast::hash(b"cell"), 0xCB87_87E2);
        let vector = hashing_vector("cell");
        assert!((vector[1009] + 0.866_025_45).abs() < 1e-6);
        // The word and its letter group "cell" share a place; "<cel" and "ell>" take two more.
        assert_eq!(vector.iter().filter(|value| **value != 0.0).count(), 3);
        let sentence = hashing_vector("Mitochondria produce most of the chemical energy that a cell needs to function.");
        let filled: Vec<(usize, f32)> = sentence.iter().copied().enumerate().filter(|(_, value)| *value != 0.0).collect();
        assert_eq!(filled.len(), 57);
        for ((place, value), (expected_place, expected)) in filled.iter().zip([(41, -0.08528), (43, -0.08528), (73, 0.08528), (104, -0.08528)]) {
            assert_eq!(*place, expected_place);
            assert!((value - expected).abs() < 1e-5);
        }
        assert!(hashing_vector("").iter().all(|value| *value == 0.0));
    }

    #[test]
    fn model_vectors_are_scaled_to_length_one_and_keyed_by_model() {
        let fake = FakeLlm::new();
        let embedder = Embedder::Model { llm: &fake, model: "embed/one:1".into() };
        assert_eq!(embedder.key(), "model-embed_one_1");
        assert_eq!(Embedder::Hashing.key(), "hashing-v1");
        let many: Vec<String> = (0..70).map(|n| "x".repeat(n + 1)).collect();
        let vectors = embedder.embed(&many).unwrap();
        assert_eq!(vectors.len(), 70);
        assert!(vectors.iter().all(|vector| (dot(vector, vector) - 1.0).abs() < 1e-5));
        fake.go_down();
        assert!(embedder.embed(&many).is_err());
    }
}
