# Architecture

Interactive Tutor is an offline-first desktop application with three processes. The backend is
being moved from Python into the shell (`docs/plans/rust-migration.md`), so for now the UI talks
to both:

```
Tauri shell (Rust) + engine  ──spawns──▶  Python backend (FastAPI, 127.0.0.1:<random port>)
   │ hosts WebView2; library, search,     │ document processing only: PyMuPDF, OCR,
   │ settings, Visualise, Ask, models     │ structure, passages, first concepts
   ▼                                      ▼
React UI  ──shell commands──▶ engine  ──▶  one SQLite file  ◀──  backend
                               └──HTTP──▶  Ollama (127.0.0.1:11434)
React UI  ──HTTP + launch token──▶ backend   (health, add a book, remove a book)
```

## Process lifecycle

- The shell creates the database if needed and applies pending migrations, then reserves a free loopback port and generates a per-launch token, then starts the
  backend with `TUTOR_PORT`, `TUTOR_API_TOKEN` and `TUTOR_WATCH_STDIN=1`.
- The UI gets the port and token from the `backend_info` Tauri command and polls `/api/health`.
- The shell kills the backend on exit. The backend also exits when its stdin closes, so a crashed
  shell cannot leave it orphaned.
- Backend output goes to `backend.log` in the Tauri app log directory.

## Decisions

| Decision | Reason |
| --- | --- |
| Python 3.12 venv, not the system 3.14 | Native ML packages lag new Python releases. |
| Launch token on the local API | CORS does not stop a website from sending simple POSTs to localhost. Without a token, any page open in a browser could reach the learner's data. |
| Raw `sqlite3` and numbered SQL migrations | The schema relies on FTS5 tables and triggers, which an ORM would obscure. |
| FTS5 external-content table with triggers | Passage text is stored once and the keyword index cannot drift from it. |
| Page numbers are 1-based PDF indices | They are always well defined. The printed number is kept separately as `pages.label`. |
| Assessment scores and evaluator confidence are separate columns | The brief requires it, and a low-confidence high score must not count as mastery. |
| Mastery is one row per concept and dimension | Exposure, recall, comprehension, application and independence are tracked independently. |
| Cloud models are excluded | Models such as `*:cloud` in Ollama forward prompts off the machine, which would upload book text. |
| System fonts only | The UI must render correctly with no network access. |

## Layout

```
backend/app/       FastAPI application
  api/             HTTP routes (no business logic)
  db/              connection, migrator, migrations/*.sql (the engine embeds the same files)
engine/core/       Rust engine: db, library, search, settings, climate, content
engine/geometry/   Rust geometry for the 3D models
frontend/src/      React UI
  api/             typed backend client and connection hook
  workspaces/      one module per workspace
frontend/src-tauri Tauri shell and backend process management
frontend/e2e/      scripts that drive the real desktop window
```

The four engines (document, knowledge, teaching, assessment) live under `backend/app/engines/`
and must not import from `app/api`, keeping the AI independent of the interface.

## Knowledge engine (`engine/core`: `answer`, `retrieve`, `grounding`, `embeddings`, `vectors`, `llm`)

```
question ─▶ subject (content words, known concepts) ─▶ FTS5 + vectors, fused ─▶ top passages ─▶ model ─▶ verify quotes
```

- The `Llm` trait is the only interface the engines use; `Ollama` implements it over plain HTTP
  (`ureq`, built without TLS, since the runtime is on this computer).
- Work that runs the model runs on a blocking thread and sends events to the window over a
  Tauri channel. Each request has an id; Stop sets a flag for that id, the engine stops reading
  the reply, and closing the connection is what stops Ollama generating.
- Vectors for a book are made the first time it is asked about and kept in
  `indexes/book-<id>-<embedder>-<passages>.vec`; the name changes when the passages do. The
  comparison is exact, against every passage, as the FAISS index it replaced was.
- Cloud models are refused inside the provider on every call, using Ollama's `remote_host` field as
  well as the name, so no setting or UI path can send book text off the machine.
- The model must label each claim as supported, background, interpretation or not found.
- Grounding is enforced in code. A "supported" claim must quote its passage, the quote is checked
  against the stored text, and a claim that fails is downgraded and flagged. Page numbers always
  come from the database, never from the model.
- A question sharing no words with the book is answered "not found" with no model call.
- Prompts are capped at 4 passages and about 5000 characters.
- `Embedder` is pluggable. An installed embedding model replaces the hashing fallback with no other
  change, and each FAISS index is keyed by embedder and passage set, so it rebuilds itself.
- The knowledge map is built at ingest with no model, from glossaries, "is called X" phrasing and
  section titles. Model enrichment is per chapter and on request, and keeps only concepts whose
  quoted evidence exists.

## Packaging (`packaging/`)

- The backend is frozen with PyInstaller as a folder rather than a single file, so it does not
  unpack itself on every launch, and is bundled as a Tauri resource alongside Tesseract.
- `build-installer.ps1` produces the NSIS installer. Ollama and models are not bundled.
- `-E2E` builds a test-only version exposing the WebView debug port. Shipped builds cannot.

## Document pipeline (`engines/documents`)

```
extract  ─▶  ocr (only pages lacking text)  ─▶  remove furniture  ─▶  structure  ─▶  segment  ─▶  SQLite + FTS5
```

- One background worker processes books in turn. OCR runs in parallel inside a book, in small
  batches to bound memory.
- Every stage works on a flat list of lines carrying their page, so page references survive to
  each passage, and chapters can start mid-page.
- Chapter detection uses the most trustworthy signal available, in order: PDF outline, "Chapter N"
  headings, numbered headings, font-size tiers, then honest page ranges. The method is stored on
  each chapter.
- Passages break only at paragraph or sentence boundaries, aiming for 900 characters with a 1600
  limit, so lengths vary with the text.
- PDFs are read into memory so Windows does not lock files against removal.
- Book title and status are written last in one transaction, so a crash leaves a book that is
  reprocessed on the next start rather than half-indexed.

## Search (`engine/core/src/search.rs`, Rust; shell commands in `src-tauri/src/library.rs`)

```
query ─▶ parse (words, "phrases", prefix on the word being typed) ─▶ FTS5 MATCH within a scope ─▶ grouped hits + concept matches
```

- User input is never passed to FTS5 as syntax: every word and phrase is quoted.
- All words must match; if nothing does, any word may, and the response says which happened.
- A scope (book, chapter, section) bounds the index scan by the passage id range it covers, since
  a book's passages are stored together, and then filters exactly. The joins are written as
  `CROSS JOIN` to fix the order (index first), because the planner otherwise sometimes probes the
  index once per passage. On a 100,000-passage test library this took a search inside one book
  from about 115 ms to 9 ms; a common word across all books takes about 120 ms.
- Matches are marked with the control characters U+0002 and U+0003, which cannot occur in
  extracted text. The UI splits on them and never renders book text as HTML.
- The UI waits 150 ms after the last keystroke and ignores the answer to a query it has replaced.
- A search from the window to the engine and back takes about 8 ms on the small test library.
- The library lists, structure, passages, collections and settings are served the same way
  (`engine/core/src/library.rs`, `settings.rs`). Each command opens its own connection; the
  database is in WAL mode, so the window reads while the backend is writing a book.

## Visualise (`engines/visual`, `api/visual.py`, `frontend/src/workspaces/library/visual`)

```
book text / pasted table / typed values ─▶ parse and validate ─▶ dataset (SQLite) ─▶ figures the data supports ─▶ interactive drawing + explanation + provenance
```

- No number is invented. Values are read from text by pattern matching or entered by the learner,
  and every derived statement (prevailing wind, daily range, position against the comfort band)
  is arithmetic on them. The model is not involved.
- A dataset records whether it was read from the book, with the passage, or entered by the learner.
  Editing a value by hand turns book data into entered data.
- The backend decides which figures the data supports and supplies each figure's statement of what
  is data and what is schematic. The UI shows that statement beside every figure.
- Explanations separate facts computed from the data from design principles, which are labelled
  as general knowledge rather than quotations from the book.
- 2D figures are hand-written SVG with keyboard-reachable parts. The 3D model uses Three.js, loaded
  only when its tab is opened. Its geometry is a fixed schematic driven by the data; shapes are
  never generated freely.

## Geometry engine (`engine/geometry`, Rust)

```
profile with holes ─▶ push along its normal ─▶ solid (faces, loops, shared vertices) ─▶ solidity and volume checks ─▶ triangles + edge lines ─▶ Three.js
```

- Modelled on the structure SketchUp exposes through its public API (`docs/research/3d-engines.md`):
  planar faces, each an outer loop with holes, sharing vertices. Triangles and edge lines are
  derived for drawing and are never the model.
- A solid must be closed: every edge used by exactly two faces, once each way. Volume is computed
  from the faces. Both are tested, so a part with a gap cannot reach the screen unnoticed.
- The shell exposes `room_model`; the window receives ready meshes and only draws them. Openings
  are real holes through the walls.
- No curved surfaces. Rhino's NURBS approach was studied and deliberately not adopted.

## Visuals from the book (`engine/core`, Rust; shell commands in `src-tauri/src/visual.rs`)

- Concept map: the book's own concepts, linked by appearing in the same passage.
- Processes and figures: found by pattern in passage text; wording and values are never altered.
- Model diagrams: the model proposes nodes with quotations; unsupported nodes are dropped and
  counted, and the count is shown (`engine/core/src/diagram.rs`).
- Climate data (`engine/core/src/climate.rs`) moved with it: parsing, validation, summaries,
  the choice of figures, and storage.
- The engine opens the same SQLite file as the Python backend, in the folder chosen the same
  way (`TUTOR_DATA_DIR`, else `%LOCALAPPDATA%\InteractiveTutor`).
