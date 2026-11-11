# Implementation Status

The application is **not complete**. Phases 1 to 3 of 7 are done, plus library search and Visualise. A Windows installer of this version (0.5.0) exists.
The backend is being moved from Python to Rust (`docs/plans/rust-migration.md`); stages 0 to 3 of 5
are done: the 3D geometry, Visualise, the database, library, search, settings, and everything
that uses the local model. Only document processing (reading PDFs, OCR, structure) is still Python.

| Phase | State |
| --- | --- |
| 1. Foundation | Done, verified in the real desktop window |
| 2. Document processing | Done, verified in the real desktop window and the installed app |
| 3. Local AI | Done, verified with the real local model in the desktop window; two-model benchmark deferred |
| 4. Teaching | Not started |
| 5. Voice | Not started |
| 6. Learning memory | Not started (tables exist) |
| 7. Packaging | Installer built early (see below); offline hardening, performance and settings remain |

## Installer

- `release/Interactive Tutor_0.5.0_x64-setup.exe`, installs per user without admin rights.
- It includes the app, the frozen Python backend and Tesseract OCR with English data.
- It needs Ollama installed separately with a local model. Models are not bundled.
- Rebuild it with `packaging\build-installer.ps1`.

## Works today

- Desktop app that starts and stops its own backend, with an authenticated loopback connection.
- My Library: upload, processing status, title search, collections, metadata, removal and resumed
  processing. Reading by chapter and section with page references.
- Search, modelled on how OneNote's search behaves (`docs/research/onenote-search.md`): results as
  you type, across all books or within a book, chapter or section; "exact phrases"; word forms
  matched by stemming; results grouped by book with their chapter, section and page; matching
  concepts listed separately; every match marked in the open section, with Previous and Next
  (F3, Shift+F3); a results pane that can be pinned beside the text; recent searches. Ctrl+F
  focuses the search box. Text read from scans is searched like any other and labelled "scanned".
- Scanned PDFs through Tesseract OCR.
- Ask: grounded answers from the local model. Each claim is labelled as from the book (with a
  verified quote and page), general knowledge, interpretation or not found. A question with no
  match in the book is not sent to the model. Stop cancels.
- Concepts: knowledge map built automatically at import, plus optional per-chapter AI enrichment
  that keeps only concepts with verified evidence.
- Visualise, drawn from the uploaded book itself. For the whole book or any chapter or section:
  a map of its concepts (linked when they are discussed in the same passage), the step-by-step
  processes its text sets out, and charts of figures its text states. Every item opens the passage
  it came from, and step wording and values are the book's own. A section can also be drawn by
  the local model, which keeps only items whose quotation is found in the text; these diagrams are
  saved with the book.
- Visualise, climate data (for building climatology): monthly
  temperatures and wind frequencies for a place, found in the book's text, pasted or typed in;
  four figures chosen by what the data supports (wind rose, temperature against a comfort band,
  a section showing air through a room, and a 3D model that rotates, comes apart and can be turned
  to face each wind); click any petal, month or building part for an explanation; every figure
  states where its data came from and what in it is schematic. Data is saved with the book.
- Settings: model selection with cloud models excluded, embedding model selection, model download
  and removal, compute preference, reading speed, explanation depth, assessment difficulty and
  storage folder.

## Not built yet

Learning Room, Progress Centre and Revision Centre show which phase builds them and have no controls.

## Known limitations

- **Hardware.** The RTX 5080 is reported as not present (`CM_PROB_PHANTOM`) while on battery, so
  `qwen3.8` runs on CPU and uses about 18 GB of system RAM. Answers take roughly 15 to 140 seconds.
  One automated run was killed by Claude Code because the system ran low on memory.
- **Stop.** Stop frees the interface immediately, but the model keeps working until its first
  token, up to about 2 minutes on CPU. A following question waits behind it.
- **Search.** It matches words and their forms, not meaning, and does not tolerate typos. Previous
  and Next step through the open section only, not on into the next one. A word being typed
  matches as a prefix of the stemmed word, so "runni" finds nothing until it becomes "running".
- **Benchmark.** Only one model has been benchmarked, by the owner's instruction not to download
  models yet. `backend/tools/evaluate.py --models a b` runs the comparison once more are installed.
- **Installed-app Ask.** In the installed app the backend received Ask requests and Ollama
  answered them, but a full uninterrupted Ask run was not completed: interference, time limits and
  memory pressure. The same code passed end to end in the desktop debug build with the real model.
- **Visualise.** What is found without the model is found by pattern: numbered or "first, then,
  finally" steps, arrowed sequences, lists of percentages and year-by-year values. Processes and
  figures written any other way are missed, as are tables printed as images. The model-drawn
  diagram has only been tested with a scripted stand-in for the model, not the real one. The app
  needs the desktop window, because the Rust engine serves the library; a browser preview cannot show it. There is no
  wind speed, season, humidity, sun path or rainfall yet. The air path in the section and 3D model
  is a drawing of the principle, not a simulation. Not built from the specification: other
  subjects, generated illustrations, flowcharts, knowledge checks, annotation, comparison, and
  learning memory.
- **Unsigned.** The installer is not code-signed, so Windows SmartScreen will warn on first run.
- **Licences.** PyMuPDF is AGPL-3.0, so distributing the installer publicly requires AGPL
  compliance or a commercial licence. Tesseract is Apache-2.0.
- **Document handling.** Code listings lose their line layout; tables and figures are not
  interpreted; OCR is English only; two-column scans are untested.
