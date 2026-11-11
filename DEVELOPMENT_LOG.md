# Development Log

## 2026-11-09: Phase 1, Foundation

**Environment:** Windows 11, RTX 5080 Laptop (16 GB, driver 621.01), 31.4 GB RAM, 63 GB free on C:,
Node 24.18, Ollama 0.40.15, WebView2 157. System Python is 3.14, so the backend uses a uv-managed
3.12 venv. Installed Rust 1.99.1 and VS 2022 Build Tools (C++ workload) with winget.

**Built:** FastAPI backend, SQLite migrations and full schema, Settings API, React and Tailwind UI
with five-workspace navigation, Tauri shell that manages the backend process.

**Problems found by running the real app:**

1. The first window test timed out. Port 9222 was already held by an unrelated application on the
   machine (D5 Launcher), so the test attached to that application instead of Tutor. A diagnostic
   script reloaded one of its windows once before this was noticed.
   Fix: the test now takes an explicit port checked to be free and refuses any window not titled
   Interactive Tutor.
2. `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` had no effect because Tauri supplies its own browser
   arguments. Fix: the window is created in code, and debug builds add the debug port when
   `TUTOR_E2E_CDP_PORT` is set. Release builds cannot expose it.

**Observation:** a venv `python.exe` is a launcher with a child interpreter, so killing it alone
would orphan the server. The stdin watch handles this; 0 backend processes remained after closing.

## 2026-11-09: Phase 2, Document Processing

**OCR decision.** Benchmarked on 4 pages of *Think Python*, scored by word recall against the true
embedded text:

| Engine | DPI | Seconds per page | Word recall |
| --- | --- | --- | --- |
| RapidOCR 1.4.4 (CPU) | 200 | 13.72 | 0.967 |
| Tesseract 5.4.0 | 150 | 0.78 | 0.988 |
| Tesseract 5.4.0 | 200 | 0.94 | 0.999 |
| Tesseract 5.4.0 | 300 | 1.43 | 0.994 |

RapidOCR also lost word spacing, which would damage search and speech. Tesseract at 200 DPI was
chosen, behind an `OcrEngine` protocol so it can be replaced.

**Real-document results.** *Think Python* (244 pages, CC BY-NC): 0.7 s, 23 chapters from the PDF
outline, 480 passages. A 60-page scan of it: 10 s, chapters detected from headings alone.
Through the real window, both PDFs together: 12.6 s.

**Problems found by running real documents, each now covered by a test:**

1. "Chapter 1" was rejected as a contents entry because it ends in a number.
2. Contents pages became false sections and teachable text, including when number, title, dots and
   page sit on separate lines and on short first or last contents pages.
3. Running heads naming the current section never repeat exactly, so repeat counting missed them.
   They are now caught by the consistent offset between PDF page and printed page number.
4. PDFs with no title metadata showed the file name although the title page had the real title.
5. Removing a book could collide with the worker reading its file (Windows sharing violation) and
   return a 500 after the row was gone. The file is now removed first, with retries.

**Mistake:** a bulk edit through PowerShell's default encoding garbled non-ASCII characters in
`structure.py`. Repaired the same session and checked; use the edit tools or explicit UTF-8.

## 2026-11-09: Phase 3, Local AI

**Constraint from the owner:** use only the already-installed `qwen3.8:latest` and download no
models until the app is built. The two-model benchmark is therefore deferred. `tools/evaluate.py`
accepts several models, so it can be rerun unchanged.

**Measured about the model:**

- `/api/embed` returns 501 for qwen3.8, so there is no embedding model. Retrieval uses a pluggable
  embedder that falls back to hashed word and character n-grams in FAISS. That matches word forms,
  not meaning.
- `size_vram` was 0 throughout: it ran entirely on CPU at about 4.7 tokens per second. Windows
  reported the RTX 5080 as `CM_PROB_PHANTOM` (not present) with the laptop on battery, though
  `nvidia-smi` worked at the start of the session. GPU performance has not been measured.

**Evaluation on *Think Python*:**

| Retrieval (20 questions) | hit@1 | hit@5 | MRR |
| --- | --- | --- | --- |
| Keyword (FTS5) | 0.30 | 0.85 | 0.51 |
| Vector (hashing) | 0.40 | 0.80 | 0.56 |
| Hybrid | 0.30 | 0.95 | 0.58 |

Many of these questions share words with the section title, so hit rates would likely be lower for
freely paraphrased questions.

Grounded answers with qwen3.8 (6 questions): 5 behaved as expected, and 5 of 5 quotes verified
against the source. All 3 questions the book cannot answer were reported as not found. The one
counted miss gave the correct answer; the gold page range was too narrow, since the glossary on
p. 69 also states it. Latency was 78-140 s with other work running on the machine, and 14-23 s
through the real window with the model loaded.

**Problems found by running the real thing, each now covered by a test:**

1. Starlette's `BaseHTTPMiddleware` hid client disconnects, so stopping an answer would have left
   the model generating. The token check is now plain ASGI middleware.
2. Pressing Stop resubmitted the question. React reused one DOM node for Stop and Ask, which became
   `type=submit` during the click. The buttons now have distinct keys.

**Known behaviour:** Stop frees the interface immediately, but Ollama sends nothing until the first
token, so the backend can only cancel generation at that point.

## 2026-11-10: Early packaging (requested by the owner before Phase 4)

**Output:** an 84 MB NSIS installer (per user, no admin rights). Installed size is 340 MB: the
frozen backend is 168 MB and Tesseract is 164 MB.

**Problems found only by testing the frozen and installed builds:**

1. Bundled Tesseract returned nothing, silently: the `tsv` output mode needs `tessdata/configs`.
   It is now bundled, and output that is not TSV is an error.
2. OCR failed on every page in the installed app only. Tauri gives resource paths in `\\?\C:\...`
   form, and Tesseract cannot find its data from that path. The prefix is now removed, and the
   first OCR error is shown in the book's failure message. Confirmed by reproducing with and
   without the prefix.

**Verified with the installed test build (same code, plus the debug port):** both PDFs processed
(the scan with OCR: 40 pages, 4 chapters), navigation and search worked, the origin was
`tauri.localhost` under the shipping CSP, and the backend stopped on close.

**Verified with the installed shipping build:** it launches, the UI reaches the bundled backend,
the debug port stays closed even when requested, and closing leaves no backend running.

**Not completed:** an uninterrupted Ask run in the installed app. The attempts were interrupted by
the owner using the window, by this tool's 590 s command limit (which closed the app), and by
Claude Code killing a run under low memory. Ask passed end to end in the debug desktop build.

## 2026-11-10: Search, modelled on OneNote

**Research:** read-only static inspection of the installed OneNote desktop binaries, recorded in
`docs/research/onenote-search.md`. Nothing was executed or decompiled and no note content was read.
The tutor adopts the observed design patterns, not the code.

**Built:** `/api/search` with scopes, phrases, live prefix matching, grouping by book, honest
totals, concept matches and recent searches (migration 0002); marked matches on
`/sections/{id}/passages?q=`; the React search box, results, pinned pane and match stepping in both
the library and the reader.

**Problems found by measuring and by running the real thing:**

1. The backend had changed its match markers from `[[ ]]` to control characters while the UI still
   split on `[[`, so highlights would have shown as raw characters. The UI now shares the markers.
2. On a 100,000-passage library, a search scoped to one section still scanned every match in the
   library (110 ms for 7 results). Bounding the scan by the scope's passage id range fixed it (6 ms).
3. A first attempt at that bound ran for 2-3 seconds: SQLite started from the passages table and
   probed the full-text index once per passage. `CROSS JOIN` now pins the join order.
4. A result found by a prefix ("osmo") opened a section with nothing marked, because marking did
   not use prefix matching. It now parses the query the same way live search does.

**Verified:** 129 backend tests and 16 frontend tests pass, and the production build succeeds. The
whole flow was driven in Edge against the real backend with two generated books on a throwaway
data folder: live search across books, opening a result in another book, stepping with the buttons
and F3, pinning, changing scope, concept matches, phrases, Ctrl+F and recent searches.

**Not verified:** the Tauri desktop window and the installed app were not rebuilt or run with this
change, and `release/` still holds the installer built before it.

## 2026-11-10: Visualise, first slice (building climatology)

**Brief:** the owner supplied a specification for a Visual Intelligence Engine and, asked what was
missing from their own material, named building climatology: wind and temperature figures with no
drawings to explain them. They chose all three mocked-up figure types and asked for 3D.

**Decisions made by the builder, not the owner:** a Visualise tab on each book rather than a new
workspace; offline, with no cloud services, image generation or model calls; Three.js bundled for
3D; a fixed schematic building rather than generated geometry; a default comfort band of 20-27 C
shown as an editable assumption. The specification's suggested stack (Next.js, Supabase, cloud
models) was not adopted because it conflicts with the app's offline design.

**Found by running the real thing:** the 3D model first said the inlet "faces" a wind 45 degrees
off it. The threshold is now about 23 degrees, and beyond that it says the wind meets the inlet at
an angle.

**Verified:** 150 backend and 23 frontend tests pass and the production build succeeds. Driven in
Edge against the real backend on a throwaway data folder: a generated PDF with a climate table was
uploaded, the table was found on its page and saved, each figure was opened and clicked, the 3D
model was rotated, turned, exploded and switched between winds, the source link opened the reader
at the passage, and the data survived a reload.

**Not verified:** the Tauri desktop window and the installed app; real textbooks (only a generated
PDF was tried, so how often real tables are recognised is unknown); 3D on this machine's GPU (the
automated run used software rendering). Three.js was not put through the Endor dependency review.

## 2026-11-10: Release 0.2.0

**Output:** `release/Interactive Tutor_0.2.0_x64-setup.exe`, 84 MB, with library search and the
first slice of Visualise. The 0.1.0 installer is kept beside it.

**Found by driving the packaged window:** straight after opening a search result, the match bar
said "No matches" until the section's marked text arrived. It now says it is still finding them.

**Verified:** the frozen backend on its own (version 0.2.0, schema 3, a PDF processed, its climate
table found, search answered). The shipping executable launches, its backend refuses requests
without the launch token, the debug port stays closed even when requested, and closing it leaves
no backend running. A test-only build of the same code was driven in the real window under the
shipping content security policy, on a throwaway data folder: search across books, opening a
result, the wind rose, the temperature chart and the 3D model, which rendered on the Intel GPU.

**Not verified:** the installer was not installed and run, since that would upgrade the owner's
real library database to schema 3. Ask was not re-run in this build. The four connection errors
logged at startup are the UI polling for the backend before it is listening.

## 2026-11-10: Visuals from the uploaded book, and the first Rust engine

**Correction from the owner:** Visualise handled only the wind and temperature case used as an
example in discussion, not the content of the uploaded book. It now draws from the book: concept
map, processes and figures for any part of it, with the climate tool kept as one option.

**Direction from the owner:** no Python except where it performs better; study SketchUp's or
Rhino's 3D engine; consider Rust. The builder advised that Python is not the bottleneck and the
owner decided to proceed. Findings are in `docs/research/3d-engines.md` and the plan in
`docs/plans/rust-migration.md`. Stage 0 is built: a Rust geometry engine that the 3D model now
uses instead of hand-placed boxes.

**Found by running the real thing:**

1. Opening Visualise on the section first shown in the reader ("Front matter") found nothing to
   draw. The whole book is now the default.
2. Points on the line chart were hard to hit. Each now has a full-height click area.

**Mistake:** an Interactive Tutor window that this session had not started was open while the
test copy was launched. The test copy crashed because both shared one WebView profile, and the
clean-up then closed the wrong process (a normal close, not a forced kill). Test copies now get
their own WebView profile and are closed only by the process ID that was launched.

**Verified:** 170 backend, 29 frontend and 12 Rust tests pass. Content visuals were driven in Edge
against the real backend with a generated book. The engine-built 3D model was driven in the real
desktop window on a throwaway data folder: 8 closed solids, with the wall openings cut through.

**Not verified:** real textbooks; the model-drawn diagram with the real model; the installer run
end to end.

## 2026-11-10: Rust migration stage 1, release 0.3.0

**Moved to Rust (`engine/core`):** climate data (reading tables from text, validation, summaries,
choice of figures, storage) and the figures found in a book (concept map, processes, charts, saved
diagrams). The window calls them as shell commands. The Python they replace is deleted, along with
its tests, whose cases were carried over first. Still Python: planning a diagram with the model.

**Decisions made by the builder:** `fancy-regex` so the text patterns could be carried over
unchanged (they use look-behind, which the standard Rust crate lacks); `rusqlite` with its own
bundled SQLite; a single engine crate that later stages will fill.

**Verified:** 132 Python, 29 frontend and 39 Rust tests pass. In the real desktop window on a
throwaway data folder and its own WebView profile: three PDFs processed; concept map, steps and
figures drawn from a book; a climate table found in a book and saved; a pasted table read, an
invalid value refused with its reason, then saved; the 3D model built by the geometry engine.
The shipping executable was rebuilt afterwards.

**Not verified:** the installer run end to end; real textbooks; behaviour when the Python backend
is writing a large book while the engine reads (both use the same database file).

## 2026-11-11: Rust migration stage 2, release 0.4.0

**Moved to Rust (`engine/core`):** opening the database and applying migrations (`db`); the
library as the window reads it: books, title filter, collections, structure, passages, pages
(`library`); search, marking of matches and recent searches (`search`); settings and their
validation (`settings`). The window calls them as shell commands. The Python routes and the Python
search engine they replace are deleted, with their tests carried over first. Still Python: adding
and removing a book, document processing, Ask, concepts, the model-planned diagram, model
management.

**Decisions made by the builder:** the shell migrates the database before starting the backend, so
the two never do it at once, and both record the same file names so either can run first; the
migration files stay where the Python tests can reach them; the Python tests that remain read the
database directly instead of through the deleted routes; an unfinished search is no longer
cancelled, only ignored, since a search takes a few milliseconds.

**Behaviour changes:** a scope with no id is refused even when the query is empty. A collection
name over 100 characters is measured after trimming. The app no longer works in a plain browser
preview at all.

**Verified:** 88 Python, 29 frontend and 82 Rust tests pass (70 engine, 12 geometry). In the real
desktop window on a throwaway data folder and its own WebView profile: the shell created and
migrated a new database; three PDFs were added and the engine listed them 13 times while the
backend was writing them, with no refusal and no read slower than 26 ms; title filter;
collections created, filled and filtered; search while typing across books, a hit opened with its
matches marked and stepped through; an exact phrase inside a book; recent searches shown and
cleared; settings saved by the engine and read back by the Python backend; every refusal worded
for the learner. The only HTTP calls the window still made were health, upload and model status.

**Not verified:** the installer run end to end; real textbooks; an existing library from 0.3.0
opened by 0.4.0 (the schema is unchanged, and the engine has a test for a Python-made database).

## 2026-11-11: Rust migration stage 3, release 0.5.0

**Moved to Rust (`engine/core`):** the Ollama client with its cloud-model refusal (`llm`);
hashing and model embeddings (`embeddings`); the per-book vector index (`vectors`); retrieval
(`retrieve`); checking quotations (`grounding`); Ask (`answer`); listing concepts and adding to
them with the model (`concepts`); the model-planned diagram (`diagram`); model status, download
and removal (`ai`); the prompts. The Python that did this is deleted, with FAISS, numpy and httpx
dropped from the backend, whose frozen size fell from 168 MB to 76 MB.

**Decisions made by the builder:** FAISS is replaced by an exact comparison in plain Rust, because
the FAISS index used was itself exact and a book has a few thousand passages; old `.faiss` files
are ignored and removed with their book; `ureq` without TLS for HTTP to the local runtime;
`crc32fast` so hashed vectors are the same numbers Python produced (checked against values printed
by the Python embedder before it was removed); progress reaches the window over a Tauri channel,
and Stop is a flag per request id rather than a dropped HTTP connection.

**Behaviour changes:** a question is trimmed before its length is checked. A model failure during
concept enrichment or drawing is one message, no longer told apart by an HTTP status.

**Verified:** 43 Python, 29 frontend and 138 Rust tests pass (126 engine, 12 geometry). In the
real desktop window on a throwaway data folder, against the Ollama on this machine (0.40.16,
qwen3.8, with one cloud model installed): the cloud model was listed as excluded and never
offered; a question sharing no word with the book was answered "not in this document" in 0.1 s
without the model; a real question was answered from the book with two verified quotations and
page references (161 s on first load, 16 s for the next question); Stop returned the panel to
idle in 1.9 s and the next question was answered promptly; concept enrichment added two concepts
with real evidence (41 s); the model drew a four-step process for a section with every step
quoted from the text and nothing dropped (46 s), and it was listed with the section; a cloud
download and removing a model that is not installed were refused in words. The only HTTP calls the
window still made were health and upload. Documents were processed by the frozen backend built
without numpy.

**Not verified:** that Ollama itself stops computing at Stop (the engine stops reading and closes
the connection, and the next question was not delayed, but the runtime was not inspected); an
embedding model, since none is installed (covered by engine tests with a stand-in); downloading
and removing a real model; the installer run end to end; real textbooks.
