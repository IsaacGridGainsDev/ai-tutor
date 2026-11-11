-- Climate data a learner visualises: monthly temperatures and wind frequencies for one place.
-- Values come from the book or from the learner; `source` records which.
CREATE TABLE climate_datasets (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('entered', 'book')),
    passage_id INTEGER REFERENCES passages (id) ON DELETE SET NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now'))
);
CREATE INDEX idx_climate_datasets_book ON climate_datasets (book_id);
