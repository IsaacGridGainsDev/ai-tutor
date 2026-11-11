# Moving the backend from Python to Rust

Decision (owner, 2026-11-10): no part of the codebase stays in Python unless Python demonstrably
performs better there. Rust was chosen over C++ by the builder: the desktop shell is already Rust,
so the backend can move into the same process, and the separate frozen Python program (168 MB of
the install) and its start-up delay go away.

## What the measurements say

Moving languages will not make the slow things fast. Answers take 15 to 140 seconds because the
language model runs on the CPU, and that engine (Ollama) is already native code. Search takes 6 to
9 ms inside a book. The gains from this move are a smaller install, a faster start, one process
instead of two, and one language for all non-interface code.

## Order of work

Each stage ends with the app working and all test suites passing. Ported code is called by the
window as a shell command instead of an HTTP route; the screens do not change. Python code is
deleted as soon as its replacement is verified, and its tests are carried over to Rust first.

Both sides open the same SQLite file. Since stage 2 the shell creates it and applies migrations
before it starts the Python backend. The migration files stay under `backend/app/db/migrations`
until stage 5, because the Python tests build databases from them; the engine embeds them at
compile time. A plain browser preview cannot reach shell commands, so the app now works only as
the desktop app.

| Stage | What moves | Notes |
| --- | --- | --- |
| 0 | Geometry engine for 3D models | **Done.** `engine/geometry`, linked into the shell. |
| 1 | Visuals: climate tables, content figures | **Done.** `engine/core`. The model-planned diagram waits for stage 3. |
| 2 | Database, library, search, settings | **Done.** `engine/core`: `db`, `library`, `search`, `settings`. Adding and removing a book wait for stage 4. |
| 3 | Model client: Ask, concept enrichment, diagrams, model management | **Done.** `engine/core`: `llm`, `embeddings`, `vectors`, `retrieve`, `grounding`, `answer`, `concepts`, `diagram`, `ai`. Events reach the window over a channel. |
| 4 | Document pipeline: extraction, structure, segmentation, OCR | The hard one. See below. |
| 5 | Remove Python, PyInstaller and the launch-token HTTP layer | Routes become shell commands. |

## Open questions for stage 4

- **Reading PDFs.** Python uses PyMuPDF, a wrapper round MuPDF, which is a C library. Rust can
  call MuPDF directly (`mupdf` crate, same AGPL terms) or PDFium (`pdfium-render`, BSD-style
  licence, which would also remove the AGPL obligation noted in the status file). The choice needs
  a test on real textbooks, because chapter detection depends on font sizes and outlines.
- **Vector search.** Done in stage 3 without FAISS: an exact comparison against every passage of
  the book, which is what the FAISS index did. Vectors are kept in `indexes/*.vec`.
- **Tests.** The Python tests describe the required behaviour and must be carried over, not
  dropped. After stage 3: 43 Python tests remain and `engine/core` has 126. The Python tests that
  remain read the database directly (`backend/tests/library_view.py`) instead of through routes.

## What is not being rewritten

The interface (TypeScript and React) and the drawing layer (Three.js on the GPU). Neither is
Python, and the 3D picture is already drawn by the graphics card.

## What is left in Python after stage 3

About 1,500 lines: receiving an uploaded PDF and removing a book (`api/library.py`), the work queue
(`engines/documents/worker.py`), reading pages (`extract.py`), OCR through Tesseract (`ocr.py`),
finding chapters and sections (`structure.py`), cutting passages (`segment.py`), the concepts found
without a model (`concepts.py`), and the HTTP server round them. All of it is stage 4.
