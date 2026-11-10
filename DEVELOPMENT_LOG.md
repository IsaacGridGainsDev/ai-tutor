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
