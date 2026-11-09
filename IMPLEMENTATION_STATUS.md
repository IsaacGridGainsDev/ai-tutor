# Implementation Status

The application is **not complete**. Phases 1 and 2 of 7 are done.

| Phase | State |
| --- | --- |
| 1. Foundation | Done, verified in the real desktop window |
| 2. Document processing | Done, verified in the real desktop window |
| 3. Local AI | Not started |
| 4. Teaching | Not started |
| 5. Voice | Not started |
| 6. Learning memory | Not started (tables exist) |
| 7. Packaging | Not started |

## Works today

- Tauri desktop window that starts and stops its own Python backend, with an authenticated
  loopback connection.
- SQLite with migrations and every table from the brief.
- My Library: upload (button or drag and drop), processing status, title search, collections,
  metadata, removal, and resuming interrupted processing after a restart.
- Reading a book: chapter and section navigation, passages with page references, and full-text
  search inside a book.
- Scanned PDFs through Tesseract OCR, page by page, including mixed documents.
- Settings: reading speed, explanation depth, assessment difficulty, compute preference and
  document storage folder persist across restarts.

## Not built yet

- Learning Room, Progress Centre and Revision Centre show which phase builds them and have no
  controls.
- From Engine 1: concept identification, the knowledge map and the FAISS index need local models
  and arrive in Phase 3.
- Resuming learning sessions from the Library needs sessions, which arrive in Phase 4.

## Known limitations

- Reading order depends on the PDF's internal order. Two-column text is tested only on a generated
  document; two-column scans are untested.
- Code listings lose their line layout in passages (for example `>>> str(32) '32' >>> ...`).
  Page text is kept, but teaching over code will need better handling.
- Tables and figures are not interpreted; only their text is extracted.
- A scan with a poor hidden text layer is trusted rather than re-recognised with OCR.
- OCR is English only.
- Chapter detection is heuristic for PDFs with no outline. The method used is stored per chapter.
- Only the debug build has been run, against the Vite dev server. The release build is untested
  until Phase 7, and Tesseract must be bundled then.

## Environment constraints for later phases

- `qwen3.8:latest` is 17 GB, larger than the 16 GB of VRAM, so it will likely split onto the CPU.
  Phase 3 must benchmark it against a smaller model.
- `deepseek-v4.1-flash:cloud` is a cloud model and must be excluded from model selection.
- About 60 GB of disk is free, so model downloads need to stay selective.
