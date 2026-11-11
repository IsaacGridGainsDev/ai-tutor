//! One file of vectors for each book and embedder, made the first time the book is asked about
//! and made again whenever its passages change.
//!
//! The Python backend used FAISS for this. Its index was an exact comparison against every
//! passage, which a plain loop does as well for the few thousand passages of a book.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::embeddings::{dot, Embedder};
use crate::{Error, Result};

const MAGIC: &[u8; 8] = b"TUTORVEC";

pub struct VectorStore {
    directory: PathBuf,
}

struct Index {
    ids: Vec<i64>,
    width: usize,
    /// Every vector, one after another.
    values: Vec<f32>,
}

fn unusable(path: &Path, error: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("The search index {} cannot be used: {error}", path.display()))
}

impl Index {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24 + self.ids.len() * 8 + self.values.len() * 4);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(self.width as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.ids.len() as u64).to_le_bytes());
        self.ids.iter().for_each(|id| bytes.extend_from_slice(&id.to_le_bytes()));
        self.values.iter().for_each(|value| bytes.extend_from_slice(&value.to_le_bytes()));
        bytes
    }

    /// None when the bytes are not a whole index, as after an interrupted write.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let number = |at: usize| Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?) as usize);
        if bytes.get(..8)? != MAGIC {
            return None;
        }
        let (width, count) = (number(8)?, number(16)?);
        let values_at = 24 + count.checked_mul(8)?;
        if bytes.len() != values_at.checked_add(count.checked_mul(width)?.checked_mul(4)?)? {
            return None;
        }
        Some(Self {
            ids: bytes[24..values_at].chunks_exact(8).map(|id| i64::from_le_bytes(id.try_into().unwrap())).collect(),
            width,
            values: bytes[values_at..].chunks_exact(4).map(|value| f32::from_le_bytes(value.try_into().unwrap())).collect(),
        })
    }
}

impl VectorStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// The file for a book's passages as they are now. Its name changes when they change.
    fn name(conn: &Connection, book_id: i64, key: &str) -> Result<Option<String>> {
        let (count, low, high): (i64, Option<i64>, Option<i64>) =
            conn.query_row("SELECT COUNT(*), MIN(id), MAX(id) FROM passages WHERE book_id = ?", [book_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(low.zip(high).map(|(low, high)| format!("book-{book_id}-{key}-{count}-{low}-{high}.vec")))
    }

    /// The files of a book, for one kind of vector or for all.
    fn files(&self, book_id: i64, key: Option<&str>) -> Vec<PathBuf> {
        let prefix = match key {
            Some(key) => format!("book-{book_id}-{key}-"),
            None => format!("book-{book_id}-"),
        };
        let Ok(entries) = fs::read_dir(&self.directory) else { return Vec::new() };
        entries.flatten().map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))).collect()
    }

    fn build(conn: &Connection, book_id: i64, embedder: &Embedder) -> Result<Index> {
        let mut statement = conn.prepare("SELECT id, text FROM passages WHERE book_id = ? ORDER BY id")?;
        let rows: Vec<(i64, String)> = statement.query_map([book_id], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let (ids, texts): (Vec<i64>, Vec<String>) = rows.into_iter().unzip();
        let vectors = embedder.embed(&texts)?;
        Ok(Index { ids, width: vectors.first().map_or(0, Vec::len), values: vectors.concat() })
    }

    fn ensure(&self, conn: &Connection, book_id: i64, embedder: &Embedder) -> Result<Option<Index>> {
        let key = embedder.key();
        let Some(name) = Self::name(conn, book_id, &key)? else { return Ok(None) };
        let path = self.directory.join(name);
        if let Some(index) = fs::read(&path).ok().and_then(|bytes| Index::from_bytes(&bytes)) {
            return Ok(Some(index));
        }
        let index = Self::build(conn, book_id, embedder)?;
        fs::create_dir_all(&self.directory).map_err(|error| unusable(&self.directory, error))?;
        for stale in self.files(book_id, Some(&key)) {
            let _ = fs::remove_file(stale);
        }
        // Written under another name first, so a half-written file is never taken for an index.
        let partial = path.with_extension("partial");
        fs::write(&partial, index.to_bytes()).and_then(|_| fs::rename(&partial, &path)).map_err(|error| unusable(&path, error))?;
        Ok(Some(index))
    }

    /// The passages of a book most like the query, the most alike first, each with its likeness.
    pub fn search(&self, conn: &Connection, book_id: i64, embedder: &Embedder, query: &str, limit: usize) -> Result<Vec<(i64, f32)>> {
        let Some(index) = self.ensure(conn, book_id, embedder)? else { return Ok(Vec::new()) };
        let wanted = embedder.embed(&[query.to_string()])?.pop().unwrap_or_default();
        if index.width == 0 || wanted.len() != index.width {
            return Ok(Vec::new());
        }
        let mut scored: Vec<(i64, f32)> = index.ids.iter().zip(index.values.chunks_exact(index.width)).map(|(id, vector)| (*id, dot(vector, &wanted))).collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Forgets a book's vectors, including any left by the Python backend.
    pub fn remove(&self, book_id: i64) {
        for file in self.files(book_id, None) {
            let _ = fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{knowledge_library, scratch_folder, KnowledgeLibrary};

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_most_alike_passage_comes_first() {
        let KnowledgeLibrary { conn, knowledge, physics } = knowledge_library();
        let folder = scratch_folder("vectors-alike");
        let store = VectorStore::new(folder.clone());
        let found = store.search(&conn, knowledge, &Embedder::Hashing, "mitochondria", 3).unwrap();
        assert_eq!(found.len(), 3);
        let text: String = conn.query_row("SELECT text FROM passages WHERE id = ?", [found[0].0], |row| row.get(0)).unwrap();
        assert!(text.contains("itochondria") && found[0].1 > found[1].1 && found[0].1 > 0.0);
        // Only that book's passages are compared.
        let other = store.search(&conn, physics, &Embedder::Hashing, "mitochondria", 50).unwrap();
        assert_eq!(other.len(), 5);
        assert!(other.iter().all(|(id, _)| found.iter().all(|(known, _)| known != id)));
        assert!(store.search(&conn, 999, &Embedder::Hashing, "mitochondria", 3).unwrap().is_empty());
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn the_index_is_kept_between_runs_and_follows_the_passages() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let folder = scratch_folder("vectors-ünïcode indexes");
        let first = VectorStore::new(folder.clone()).search(&conn, knowledge, &Embedder::Hashing, "mitochondria", 3).unwrap();
        let files = names(&folder);
        assert_eq!(files.len(), 1);
        assert!(files[0].starts_with(&format!("book-{knowledge}-hashing-v1-9-")) && files[0].ends_with(".vec"));
        // A second run reads the file: emptying the passages' text would change a rebuilt answer.
        let written = fs::metadata(folder.join(&files[0])).unwrap().modified().unwrap();
        assert_eq!(VectorStore::new(folder.clone()).search(&conn, knowledge, &Embedder::Hashing, "mitochondria", 3).unwrap(), first);
        assert_eq!(fs::metadata(folder.join(&files[0])).unwrap().modified().unwrap(), written);

        conn.execute("DELETE FROM passages WHERE id = (SELECT MAX(id) FROM passages WHERE book_id = ?)", [knowledge]).unwrap();
        VectorStore::new(folder.clone()).search(&conn, knowledge, &Embedder::Hashing, "mitochondria", 3).unwrap();
        let after = names(&folder);
        assert_eq!(after.len(), 1);
        assert_ne!(after, files);
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_damaged_index_is_made_again() {
        let KnowledgeLibrary { conn, knowledge, .. } = knowledge_library();
        let folder = scratch_folder("vectors-damaged");
        let store = VectorStore::new(folder.clone());
        let first = store.search(&conn, knowledge, &Embedder::Hashing, "gravity", 2).unwrap();
        let file = folder.join(&names(&folder)[0]);
        let whole = fs::read(&file).unwrap();
        fs::write(&file, &whole[..whole.len() / 2]).unwrap();
        assert_eq!(store.search(&conn, knowledge, &Embedder::Hashing, "gravity", 2).unwrap(), first);
        assert_eq!(fs::read(&file).unwrap().len(), whole.len());
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn removing_a_book_removes_its_vectors_of_every_kind() {
        let KnowledgeLibrary { conn, knowledge, physics } = knowledge_library();
        let folder = scratch_folder("vectors-removed");
        let store = VectorStore::new(folder.clone());
        store.search(&conn, knowledge, &Embedder::Hashing, "energy", 2).unwrap();
        store.search(&conn, physics, &Embedder::Hashing, "energy", 2).unwrap();
        // An index the Python backend left behind.
        fs::write(folder.join(format!("book-{knowledge}-hashing-v1-9-1-9.faiss")), b"old").unwrap();
        store.remove(knowledge);
        let left = names(&folder);
        assert_eq!(left.len(), 1);
        assert!(left[0].starts_with(&format!("book-{physics}-")));
        VectorStore::new(folder.join("never made")).remove(knowledge);
        let _ = fs::remove_dir_all(folder);
    }
}
