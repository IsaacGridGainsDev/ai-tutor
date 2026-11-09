-- Library -------------------------------------------------------------------

CREATE TABLE collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE books (
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    author TEXT,
    file_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    file_hash TEXT NOT NULL UNIQUE,
    page_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'extracting', 'ocr', 'structuring', 'indexing', 'ready', 'failed')),
    status_detail TEXT,
    progress REAL NOT NULL DEFAULT 0,
    needs_ocr INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_opened_at TEXT
);
CREATE INDEX idx_books_title ON books (title COLLATE NOCASE);

CREATE TABLE book_collections (
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    collection_id INTEGER NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, collection_id)
);
CREATE INDEX idx_book_collections_collection ON book_collections (collection_id);

-- Page numbers are 1-based PDF page indices; label is the printed page number if the PDF has one.
CREATE TABLE pages (
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL,
    label TEXT,
    text TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT 'text' CHECK (source IN ('text', 'ocr', 'none')),
    PRIMARY KEY (book_id, page_number)
);

CREATE TABLE chapters (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_page INTEGER NOT NULL,
    end_page INTEGER NOT NULL,
    detection_method TEXT NOT NULL,
    UNIQUE (book_id, ordinal)
);

CREATE TABLE sections (
    id INTEGER PRIMARY KEY,
    chapter_id INTEGER NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_page INTEGER NOT NULL,
    end_page INTEGER NOT NULL,
    UNIQUE (chapter_id, ordinal)
);

CREATE TABLE passages (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    chapter_id INTEGER NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    section_id INTEGER NOT NULL REFERENCES sections (id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    text TEXT NOT NULL,
    start_page INTEGER NOT NULL,
    end_page INTEGER NOT NULL,
    UNIQUE (section_id, ordinal)
);
CREATE INDEX idx_passages_book ON passages (book_id);
CREATE INDEX idx_passages_chapter ON passages (chapter_id);

CREATE VIRTUAL TABLE passages_fts USING fts5 (
    text,
    content = 'passages',
    content_rowid = 'id',
    tokenize = 'porter unicode61'
);
CREATE TRIGGER passages_ai AFTER INSERT ON passages BEGIN
    INSERT INTO passages_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER passages_ad AFTER DELETE ON passages BEGIN
    INSERT INTO passages_fts (passages_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER passages_au AFTER UPDATE OF text ON passages BEGIN
    INSERT INTO passages_fts (passages_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO passages_fts (rowid, text) VALUES (new.id, new.text);
END;

-- Knowledge map ---------------------------------------------------------------

CREATE TABLE concepts (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    definition TEXT,
    importance REAL NOT NULL DEFAULT 0,
    UNIQUE (book_id, normalized_name)
);

CREATE TABLE passage_concepts (
    passage_id INTEGER NOT NULL REFERENCES passages (id) ON DELETE CASCADE,
    concept_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    PRIMARY KEY (passage_id, concept_id)
);
CREATE INDEX idx_passage_concepts_concept ON passage_concepts (concept_id);

CREATE TABLE concept_relations (
    source_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    target_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    relation TEXT NOT NULL,
    PRIMARY KEY (source_id, target_id, relation)
);
CREATE INDEX idx_concept_relations_target ON concept_relations (target_id);

-- Learning --------------------------------------------------------------------

CREATE TABLE learning_sessions (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    chapter_id INTEGER REFERENCES chapters (id) ON DELETE SET NULL,
    section_id INTEGER REFERENCES sections (id) ON DELETE SET NULL,
    passage_id INTEGER REFERENCES passages (id) ON DELETE SET NULL,
    concept_id INTEGER REFERENCES concepts (id) ON DELETE SET NULL,
    stage TEXT NOT NULL DEFAULT 'introduction',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'completed')),
    teaching_state TEXT NOT NULL DEFAULT '{}',
    active_seconds INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_active_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT
);
CREATE INDEX idx_sessions_book ON learning_sessions (book_id, last_active_at);

-- One row per conversational turn.
CREATE TABLE conversations (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES learning_sessions (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('tutor', 'learner', 'system')),
    kind TEXT NOT NULL DEFAULT 'message',
    stage TEXT,
    content TEXT NOT NULL,
    citations TEXT NOT NULL DEFAULT '[]',
    modality TEXT NOT NULL DEFAULT 'text' CHECK (modality IN ('text', 'voice')),
    interrupted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_conversations_session ON conversations (session_id, id);

CREATE TABLE questions (
    id INTEGER PRIMARY KEY,
    session_id INTEGER REFERENCES learning_sessions (id) ON DELETE SET NULL,
    concept_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    passage_id INTEGER REFERENCES passages (id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    level TEXT NOT NULL CHECK (level IN ('recall', 'comprehension', 'application', 'independence')),
    difficulty TEXT NOT NULL DEFAULT 'standard',
    requirements TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_questions_concept ON questions (concept_id);

CREATE TABLE answers (
    id INTEGER PRIMARY KEY,
    question_id INTEGER NOT NULL REFERENCES questions (id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES learning_sessions (id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    modality TEXT NOT NULL DEFAULT 'text' CHECK (modality IN ('text', 'voice')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_answers_question ON answers (question_id);

-- Scores are 0-1. Confidence is the evaluator's certainty, stored apart from the scores.
CREATE TABLE assessments (
    id INTEGER PRIMARY KEY,
    answer_id INTEGER NOT NULL REFERENCES answers (id) ON DELETE CASCADE,
    concept_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    factual REAL NOT NULL,
    conceptual REAL NOT NULL,
    completeness REAL NOT NULL,
    reasoning REAL NOT NULL,
    application REAL NOT NULL,
    overall REAL NOT NULL,
    confidence REAL NOT NULL,
    evidence TEXT NOT NULL DEFAULT '[]',
    feedback TEXT NOT NULL DEFAULT '',
    evaluator_model TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_assessments_concept ON assessments (concept_id, created_at);

CREATE TABLE mastery_records (
    concept_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    dimension TEXT NOT NULL
        CHECK (dimension IN ('exposure', 'recall', 'comprehension', 'application', 'independence')),
    score REAL NOT NULL DEFAULT 0,
    confidence REAL NOT NULL DEFAULT 0,
    evidence_count INTEGER NOT NULL DEFAULT 0,
    last_evidence_at TEXT,
    PRIMARY KEY (concept_id, dimension)
);

CREATE TABLE misconceptions (
    id INTEGER PRIMARY KEY,
    concept_id INTEGER NOT NULL REFERENCES concepts (id) ON DELETE CASCADE,
    answer_id INTEGER REFERENCES answers (id) ON DELETE SET NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'resolved')),
    times_seen INTEGER NOT NULL DEFAULT 1,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    resolved_at TEXT
);
CREATE INDEX idx_misconceptions_concept ON misconceptions (concept_id, status);

CREATE TABLE revision_schedules (
    concept_id INTEGER PRIMARY KEY REFERENCES concepts (id) ON DELETE CASCADE,
    ease REAL NOT NULL DEFAULT 2.5,
    interval_days REAL NOT NULL DEFAULT 0,
    repetitions INTEGER NOT NULL DEFAULT 0,
    lapses INTEGER NOT NULL DEFAULT 0,
    due_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_reviewed_at TEXT
);
CREATE INDEX idx_revision_due ON revision_schedules (due_at);

-- Configuration ---------------------------------------------------------------

CREATE TABLE model_configurations (
    id INTEGER PRIMARY KEY,
    role TEXT NOT NULL CHECK (role IN ('llm', 'embedding', 'stt', 'tts')),
    provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    parameters TEXT NOT NULL DEFAULT '{}',
    benchmark TEXT,
    is_active INTEGER NOT NULL DEFAULT 0,
    UNIQUE (role, provider, model_id)
);
CREATE UNIQUE INDEX idx_model_one_active ON model_configurations (role) WHERE is_active = 1;

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
