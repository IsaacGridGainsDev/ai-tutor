# Architecture

Interactive Tutor is an offline-first desktop application with three processes:

```
Tauri shell (Rust)  ──spawns──▶  Python backend (FastAPI, 127.0.0.1:<random port>)
   │ hosts WebView2                  │ SQLite, FAISS, PyMuPDF
   ▼                                 ▼
React UI  ──HTTP + launch token──▶  API  ──HTTP──▶  Ollama (127.0.0.1:11434)
```

## Process lifecycle

- The shell reserves a free loopback port and generates a per-launch token, then starts the
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
  db/              connection, migrator, migrations/*.sql
frontend/src/      React UI
  api/             typed backend client and connection hook
  workspaces/      one module per workspace
frontend/src-tauri Tauri shell and backend process management
frontend/e2e/      scripts that drive the real desktop window
```

The four engines (document, knowledge, teaching, assessment) live under `backend/app/engines/`
and must not import from `app/api`, keeping the AI independent of the interface.

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
