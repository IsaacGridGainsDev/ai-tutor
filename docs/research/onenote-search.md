# How OneNote desktop search works: static evidence

Target: Microsoft OneNote desktop 16.0.20527.20150, the Click-to-Run install on this machine.
Method: read-only static inspection. OneNote was not executed or decompiled, because no
disassembler engine (Ghidra, IDA or Java) is installed. Evidence comes from REA artifact identity
records, a PE parser (`pefile`), bounded string extraction and registry reads. No note content was
read.

## Artifact identities

| File | SHA-256 | REA Evidence |
| --- | --- | --- |
| `ONENOTE.EXE` | `843e46ee1f5e13b9e6f62dbf28e49e687345e4e12b1347274ef71fad7d6e761f` | — |
| `onmain.DLL` (OneNote Main, 24.8 MB) | `35b57a83aa66bca0c48a5e0421f00bbbe7680bd59cbb24bbac298ecfc19b023a` | `ev_1ee46fbe…` (managed check) |
| `ONFILTER.DLL` (OneNote Filter) | `3a08c3e0c109368a8e5a3dc70c3d9889036b4ab69a3599bdf0890805cc3caf3d` | `ev_a16a8991…` (artifact), `ev_e565f620…` (managed check) |
| `MSOCR.DLL` (Microsoft OCR Library) | `76c7421aea74283d…` | — |

REA's managed check found no CLI data directory in either DLL, so they are native x86-64 code
rather than .NET. Offsets below are file offsets in `onmain.DLL` unless stated otherwise.

## Observations

### 1. Its own SQLite full-text index

| Offset | String |
| --- | --- |
| `0x111c5c0` | `CREATE VIRTUAL TABLE PageElementContents USING fts4(tokenize=simple '' ' *-', Terms)` |
| `0x111c620` | `CREATE VIRTUAL TABLE EntityContents USING fts4(tokenize=simple '' ' *-', Terms)` |
| `0x111c560` | `CREATE VIRTUAL TABLE NoteFlagContents USING fts4(...)` |
| `0x111c500` | `CREATE VIRTUAL TABLE HashtagContents USING fts4(..., Hashtag)` |
| `0x1217890` | `PageElements.rowid IN (SELECT PageElementContents.rowid FROM PageElementContents WHERE PageElementContents.Terms MATCH ` |
| `0x1217a10` | `Entities.rowid IN (SELECT EntityContents.rowid FROM EntityContents WHERE EntityContents.Terms MATCH ` |
| `0x111bae0` | `SELECT IndexedRID, PendingIndexRID, IsResetting FROM Master WHERE rowid = 1` |
| `0x10f12c8`, `0x11028b8`, `0x10f57c8` | Feature gates `SQLite.SearchEnabled`, `SQLite.IndexingEnabled`, `SQLite.IndexActivePage` |
| `0x12558d0`, `0x111bb80`, `0x12248d0` | `SqlIndexDeltaGrep`, `SqlIndexResettingIndex`, `SqlIndexNotebookSchemaOutdated` |

### 2. Windows Search as a second, older path

Registry: the Click-to-Run class registry maps three classes to `ONFILTER.DLL`.

- `{6EE84065-…}`: "Microsoft OneNote Windows Desktop Search IFilter"
- `{C7DFFDF1-…}`: "… Windows Desktop Search Protocol Handler"
- `{0875DCB6-…}`: "… Namespace Extension for Windows Desktop Search"

Query template fragments stored next to each other, from `0x1285a48` to `0x1285b50`:

```
SELECT  … FROM SystemIndex..SCOPE() WHERE  … ("Scope" = ' … ') ORDER BY WorkId ASC
SET PROPERTYNAME ' … ' PROPID  …  CONTAINS(  …  System.ItemPathDisplay, System.Title
```

Nearby are diagnostics `ErrIndex_WDSIndexerIsDead`, `WDSIndexingDisabled` and
`Can not switch from manual to automatic indexing`. `ONFILTER.DLL` contains
`CLSID\.one\PersistentHandler\`. The Windows Search service (`WSearch`) is running.

### 3. Tokenising and word forms

`Windows.Data.Text.WordsSegmenter` at `0x1232bf0`, used through its WinRT factory
`CreateWithLanguage`. Also: `WordsSegmenterCreateFailedFallbackLocale`,
`SQLite.WordBreakerStripPunctuation`, `SQLite.SplitFtsTermOnDisallowedChars`,
`StemmerGenerateWordFormsFailed` and `FailedToStartSearchBecauseOfWordBreakerException`.

### 4. Live, asynchronous search and query parsing

- `StartLiveSearch` and `QueryToLiveSearchCondition`, in the namespace `Office::OneNote::LiveSearch`
- `IAsyncSearch::CreateInstance(ISearchResultDisplay*, ISearchPlugin*, IQueryScope*, …)`
- `ISearchQueryParser::Create`, `hNullQueryParser`, and the properties `SearchQuery` and
  `SearchQueryExact`

### 5. Scopes and hits on the current page

From `ONENOTE.EXE`: `SelectSearchScope`, `SearchScopeThisSectionGroup`, `DefaultSearchScope`,
`NoteFlagSearchScope` and `SearchScopeMenuDismissedBeforeSearch`.

From `onmain.DLL`: `FindHitsOnCurrentPage` and `FindCurrentPageHit`.

### 6. Results presentation and memory

- `SearchResultPathInName`, `SearchResultGroupLabelInName`, `PinSearchResults`,
  `PinSearchResultsFocusVisible`
- `RecentSearchResultQueryTerms`, `RecentSearchResultTitle`, `RecentSearchResultSectionGOSID_*`,
  `RecentSearchResultsSearch`
- `CEntityHit` with `EntityType`, `Content` and `Color`

### 7. Text in images is recognised

`onmain.DLL` imports `MsOcr.dll!CreateOcrEngine`. It defines the page properties `NeedsOCR`,
`HasOCRResults` and `DoNotRunBackgroundOCR`, and the feature flag `UseAugLoopOCR`.

### 8. Tags and hashtags have their own indexes

`NoteTagSearchList`, `NoteTagSearchState`, `ForceNoteTagSearch` and `IsSQLiteNoteTagSearchEnabled`.
The `Hashtags` and `HashtagContents` tables have triggers, and the queries
` WHERE HashtagContents.Hashtag MATCH ` and `SqlIndexHashtagExtractionFailed` exist.

### 9. A newer Windows content index is declared

The `ONENOTE.EXE` manifest declares `Microsoft.Windows.Search.AppContentIndex` classes:
`AppContentIndexer`, `AppIndexImageQuery` and `AppIndexQueryMatch`.

## Inferences

These follow from the strings above, but no code was traced.

- Notebooks are crawled page by page (`IndexSection`, `ForEachPage`, `PageCrawler*`), and updated
  incrementally (`DeltaGrep`, `IndexedRID`/`PendingIndexRID`).
- Page text and entity names go into FTS4 tables, and queries are SQL `MATCH` conditions that
  select page elements and entities.
- Words are split by a language-aware segmenter, and a stemmer generates word forms so that related
  forms of a word match.
- Queries are parsed into conditions, with an exact-phrase mode, and run asynchronously as the user
  types. Results go to a display callback.
- A search runs within a chosen scope, such as a section group, a notebook or the default scope.
  Hits on the open page can be stepped through.
- Results are labelled with their location path and grouped. The query terms and opened results
  are remembered as recent searches, and the results pane can be pinned.
- Text recognised in images is stored on the page and is therefore probably searchable.
- The Windows Search path probably serves the older index and shell-wide search of `.one` files.
  Which path is active is decided at run time by server-controlled feature gates.

## Unknowns

- Which engine is active on this machine and account. The gates are `TriggeredFeatureGate`s.
- How results are ranked, and whether any fuzzy or typo-tolerant matching exists.
- Exact exact-phrase and prefix semantics, and how scopes map to SQL.
- Whether, and how, the `AppContentIndex` classes are used.
- Whether this Windows Search index currently contains OneNote content. Not queried, to avoid
  reading note data.

Confirming any of these would need disassembly or runtime observation, which this investigation
did not do.

## How this informed Interactive Tutor

The tutor adopts the observed design patterns, not the code:

- one local SQLite full-text index (the tutor uses FTS5 with Porter stemming)
- live search as you type, with cancellation
- a query parser with exact phrases
- selectable scopes
- results grouped and labelled with their path
- stepping through hits in the open text
- recent searches and a pinnable results pane
- OCR text treated like any other text
- a separate concept search, in the role OneNote gives tag search
