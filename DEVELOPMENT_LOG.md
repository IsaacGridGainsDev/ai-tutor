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
