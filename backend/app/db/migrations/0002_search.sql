-- Recent searches, remembered when the learner opens a result or submits a query.
CREATE TABLE recent_searches (
    id INTEGER PRIMARY KEY,
    query TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('all', 'book', 'chapter', 'section')),
    book_id INTEGER REFERENCES books (id) ON DELETE CASCADE,
    result_title TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now'))
);
CREATE INDEX idx_recent_searches_created ON recent_searches (created_at);
