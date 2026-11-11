-- Diagrams of a section planned by the local model. Only items whose quotation was found in the
-- section's text are stored; `data` holds the nodes, the links and how many items were dropped.
CREATE TABLE diagrams (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    section_id INTEGER NOT NULL REFERENCES sections (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('process', 'hierarchy', 'comparison', 'cause_effect')),
    title TEXT NOT NULL,
    data TEXT NOT NULL,
    model TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now'))
);
CREATE INDEX idx_diagrams_section ON diagrams (section_id);
