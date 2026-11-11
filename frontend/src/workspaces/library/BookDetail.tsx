import { useEffect, useMemo, useRef, useState } from "react";
import type {
  ApiClient,
  Book,
  Chapter,
  Citation,
  Passage,
  ScopeKind,
  SearchScope,
  Section,
} from "../../api/client";
import { AskPanel } from "./AskPanel";
import { ConceptsPanel } from "./ConceptsPanel";
import { VisualisePanel } from "./visual/VisualisePanel";
import { type ScopeOption, SearchBox, SearchResults, pages, splitMarks, usePinned, useSearch } from "./Search";

/** Where a search result points, so a book can open straight at it. */
export interface OpenTarget {
  sectionId: number;
  passageId: number;
  query: string;
}

interface Props {
  client: ApiClient;
  book: Book;
  onBack: () => void;
  /** Open at this result instead of the first section. */
  initial?: OpenTarget;
  /** A result in another book was chosen. */
  onOpenBook?: (bookId: number, target: OpenTarget) => void;
}

interface Piece {
  text: string;
  /** Position among the section's matches, or null for ordinary text. */
  mark: number | null;
}

/** Lays passages out as paragraphs and numbers every match, so matches can be stepped through. */
function layOut(passages: Passage[]) {
  let total = 0;
  const first = new Map<number, number>();
  const items = passages.map((passage) => {
    const paragraphs: Piece[][] = [[]];
    for (const segment of splitMarks(passage.highlighted ?? passage.text)) {
      segment.text.split("\n\n").forEach((part, index) => {
        if (index > 0) paragraphs.push([]);
        if (!part) return;
        let mark: number | null = null;
        if (segment.marked) {
          mark = total++;
          if (!first.has(passage.id)) first.set(passage.id, mark);
        }
        paragraphs[paragraphs.length - 1].push({ text: part, mark });
      });
    }
    return { passage, paragraphs: paragraphs.filter((pieces) => pieces.length > 0) };
  });
  return { items, total, first };
}

export function BookDetail({ client, book, onBack, initial, onOpenBook }: Props) {
  const [chapters, setChapters] = useState<Chapter[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [section, setSection] = useState<Section | null>(null);
  const [passages, setPassages] = useState<Passage[]>([]);
  const [focusPassage, setFocusPassage] = useState<number | null>(initial?.passageId ?? null);
  /** The query whose matches are marked in the open section. */
  const [findQuery, setFindQuery] = useState(initial?.query ?? "");
  const [current, setCurrent] = useState(0);
  /** The section and query the loaded passages were marked for. */
  const [markedFor, setMarkedFor] = useState("");
  const [scopeKind, setScopeKind] = useState<ScopeKind>(initial ? "all" : "book");
  const [resultsOpen, setResultsOpen] = useState(false);
  const [pinned, setPinned] = usePinned();
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"read" | "ask" | "concepts" | "visualise">("read");
  const focused = useRef<HTMLElement>(null);
  const currentMark = useRef<HTMLElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);

  const chapterId = section && chapters.find((item) => item.sections.some((entry) => entry.id === section.id))?.id;
  const scope: SearchScope =
    scopeKind === "all"
      ? { kind: "all" }
      : scopeKind === "chapter" && chapterId
        ? { kind: "chapter", chapter_id: chapterId }
        : scopeKind === "section" && section
          ? { kind: "section", section_id: section.id }
          : { kind: "book", book_id: book.id };
  const search = useSearch(client, scope, initial?.query ?? "");
  const scopes: ScopeOption[] = [
    { kind: "book", label: "This book" },
    ...(chapterId ? [{ kind: "chapter", label: "This chapter" } as const] : []),
    ...(section ? [{ kind: "section", label: "This section" } as const] : []),
    { kind: "all", label: "All books" },
  ];

  useEffect(() => {
    client.bookStructure(book.id).then((result) => {
      setChapters(result);
      const wanted = (item: Section) => (initial ? item.id === initial.sectionId : item.passage_count > 0);
      const chapter =
        result.find((item) => item.sections.some(wanted)) ??
        result.find((item) => item.sections.some((entry) => entry.passage_count > 0));
      if (chapter) {
        setExpanded(chapter.id);
        setSection(chapter.sections.find(wanted) ?? chapter.sections.find((item) => item.passage_count > 0) ?? null);
      }
    }, (reason) => setError(String(reason)));
    // `initial` only matters when the book opens; the parent remounts this component for a new target.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, book.id]);

  useEffect(() => {
    if (!section) return;
    let stale = false;
    client.sectionPassages(section.id, findQuery).then(
      (result) => {
        if (stale) return;
        setPassages(result);
        setMarkedFor(`${section.id}:${findQuery}`);
      },
      (reason) => setError(String(reason)),
    );
    return () => {
      stale = true;
    };
  }, [client, section, findQuery]);

  const reader = useMemo(() => layOut(passages), [passages]);
  // Until the marked text for this section and query arrives, the match count is not yet known.
  const marksReady = section !== null && markedFor === `${section.id}:${findQuery}`;

  useEffect(() => {
    setCurrent((focusPassage !== null && reader.first.get(focusPassage)) || 0);
  }, [reader, focusPassage]);

  useEffect(() => {
    // Optional call: scrollIntoView is missing in the test DOM.
    (currentMark.current ?? focused.current)?.scrollIntoView?.({ block: "center" });
  }, [reader, current, focusPassage]);

  function step(delta: number) {
    if (reader.total > 0) setCurrent((value) => (value + delta + reader.total) % reader.total);
  }

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (view !== "read") return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        searchInput.current?.focus();
        searchInput.current?.select();
      } else if (event.key === "F3" && reader.total > 0) {
        event.preventDefault();
        step(event.shiftKey ? -1 : 1);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function changeQuery(next: string) {
    search.setQuery(next);
    setResultsOpen(next.trim() !== "");
    if (!next.trim()) setFindQuery("");
  }

  function remember(resultTitle: string | null) {
    client.rememberSearch(search.query.trim(), scope, resultTitle).catch(() => undefined);
  }

  function goTo(sectionId: number, passageId: number, query: string) {
    const chapter = chapters.find((item) => item.sections.some((entry) => entry.id === sectionId));
    const target = chapter?.sections.find((entry) => entry.id === sectionId);
    if (!chapter || !target) return;
    setExpanded(chapter.id);
    setFocusPassage(passageId);
    setFindQuery(query);
    setSection(target);
    setResultsOpen(false);
  }

  function openResult(bookId: number, sectionId: number, passageId: number, title: string) {
    const query = search.query.trim();
    remember(title);
    if (bookId !== book.id) onOpenBook?.(bookId, { sectionId, passageId, query });
    else goTo(sectionId, passageId, query);
  }

  function openCitation(citation: Citation) {
    goTo(citation.section_id, citation.passage_id, "");
    setView("read");
  }

  const searching = view === "read" && search.query.trim() !== "";
  const results = searching && (resultsOpen || pinned) && (
    <SearchResults
      result={search.result}
      pending={search.pending}
      compact={pinned}
      activePassageId={focusPassage}
      onOpen={(hit) => openResult(hit.book_id, hit.section_id, hit.passage_id, hit.section_title)}
      onOpenConcept={(concept) => openResult(concept.book_id, concept.section_id, concept.passage_id, concept.name)}
    />
  );

  return (
    <div>
      <button type="button" onClick={onBack} className="mb-4 text-sm text-muted hover:text-accent">
        ← Library
      </button>
      <h2 className="font-serif text-2xl">{book.title}</h2>
      <p className="mt-1 text-xs text-muted">
        {[book.author, `${book.page_count} pages`, `${book.chapter_count} chapters`, `${book.passage_count} passages`]
          .filter(Boolean)
          .join(" · ")}
      </p>

      <div role="tablist" aria-label="Book views" className="mt-5 mb-5 flex gap-1 border-b border-rule">
        {(["read", "ask", "concepts", "visualise"] as const).map((item) => (
          <button
            key={item}
            type="button"
            role="tab"
            aria-selected={view === item}
            onClick={() => setView(item)}
            className={`-mb-px border-b-2 px-3 py-2 text-sm capitalize ${
              view === item ? "border-accent text-accent" : "border-transparent text-muted hover:text-ink"
            }`}
          >
            {item}
          </button>
        ))}
      </div>

      {view === "read" && (
        <div className="my-5">
          <SearchBox
            client={client}
            label="Search inside this book"
            query={search.query}
            onQuery={changeQuery}
            onCommit={() => {
              remember(null);
              setResultsOpen(true);
            }}
            scopes={scopes}
            scope={scope.kind}
            onScope={(kind) => {
              setScopeKind(kind);
              setResultsOpen(search.query.trim() !== "");
            }}
            pinned={pinned}
            onPin={setPinned}
            inputRef={searchInput}
          />
        </div>
      )}

      {(error ?? search.error) && <p role="alert" className="mb-4 text-sm text-danger">{error ?? search.error}</p>}

      {/* Kept mounted so a running answer survives switching to the reader to check a citation. */}
      <div hidden={view !== "ask"}>
        <AskPanel client={client} bookId={book.id} onOpenCitation={openCitation} />
      </div>

      {view === "concepts" && <ConceptsPanel client={client} bookId={book.id} chapters={chapters} />}

      {view === "visualise" && (
        <VisualisePanel
          client={client}
          bookId={book.id}
          chapters={chapters}
          section={section}
          onOpenSource={(sectionId, passageId) => {
            goTo(sectionId, passageId, "");
            setView("read");
          }}
        />
      )}

      {view !== "read" ? null : results && !pinned ? (
        results
      ) : (
        <div className="grid grid-cols-[18rem_minmax(0,1fr)] gap-10">
          {results ? (
            <aside aria-label="Pinned search results" className="text-sm">
              {results}
            </aside>
          ) : (
            <nav aria-label="Chapters">
              <ul className="space-y-1 text-sm">
                {chapters.map((chapter) => (
                  <li key={chapter.id}>
                    <button
                      type="button"
                      aria-expanded={expanded === chapter.id}
                      onClick={() => setExpanded(expanded === chapter.id ? null : chapter.id)}
                      className="flex w-full justify-between gap-3 rounded-md px-2 py-1.5 text-left hover:bg-rule/50"
                    >
                      <span>{chapter.title}</span>
                      <span className="shrink-0 text-xs text-muted">{chapter.start_page}</span>
                    </button>
                    {expanded === chapter.id && (
                      <ul className="ml-2 border-l border-rule pl-2">
                        {chapter.sections.map((item) => (
                          <li key={item.id}>
                            <button
                              type="button"
                              disabled={item.passage_count === 0}
                              aria-current={section?.id === item.id ? "true" : undefined}
                              onClick={() => {
                                setFocusPassage(null);
                                setSection(item);
                              }}
                              className={`flex w-full justify-between gap-3 rounded-md px-2 py-1 text-left text-xs disabled:opacity-40 ${
                                section?.id === item.id ? "bg-accent-soft text-accent" : "hover:bg-rule/50"
                              }`}
                            >
                              <span>{item.title}</span>
                              <span className="shrink-0 text-muted">{item.start_page}</span>
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </li>
                ))}
              </ul>
            </nav>
          )}

          <article aria-label="Passages" className="max-w-2xl">
            {section && <h3 className="mb-4 font-serif text-xl">{section.title}</h3>}

            {findQuery && (
              <div
                role="toolbar"
                aria-label="Matches in this section"
                className="sticky top-0 z-10 mb-4 flex flex-wrap items-center gap-3 rounded-md border border-rule bg-surface px-3 py-2 text-xs"
              >
                <span role="status" className="text-muted">
                  {!marksReady
                    ? `Finding “${findQuery}” in this section…`
                    : reader.total > 0
                    ? `Match ${current + 1} of ${reader.total} for “${findQuery}”`
                    : `No matches for “${findQuery}” in this section`}
                </span>
                {reader.total > 1 && (
                  <>
                    <button type="button" onClick={() => step(-1)} className="hover:text-accent" title="Shift+F3">
                      ‹ Previous
                    </button>
                    <button type="button" onClick={() => step(1)} className="hover:text-accent" title="F3">
                      Next ›
                    </button>
                  </>
                )}
                {!pinned && searching && (
                  <button type="button" onClick={() => setResultsOpen(true)} className="hover:text-accent">
                    Back to results
                  </button>
                )}
                <button type="button" onClick={() => setFindQuery("")} className="ml-auto text-muted hover:text-ink">
                  Clear marks
                </button>
              </div>
            )}

            {reader.items.map(({ passage, paragraphs }) => (
              <section
                key={passage.id}
                ref={passage.id === focusPassage ? focused : undefined}
                data-testid="passage"
                className={`mb-6 ${passage.id === focusPassage ? "rounded-md bg-accent-soft p-3" : ""}`}
              >
                <p className="mb-1 text-xs text-muted">{pages(passage.start_page, passage.end_page)}</p>
                <div className="space-y-2 font-serif leading-relaxed">
                  {paragraphs.map((pieces, index) => (
                    <p key={index}>
                      {pieces.map((piece, position) =>
                        piece.mark === null ? (
                          <span key={position}>{piece.text}</span>
                        ) : (
                          <mark
                            key={position}
                            ref={piece.mark === current ? currentMark : undefined}
                            aria-current={piece.mark === current ? "true" : undefined}
                            className={
                              piece.mark === current
                                ? "rounded-sm bg-accent text-white"
                                : "rounded-sm bg-[#f3e3a2] text-ink"
                            }
                          >
                            {piece.text}
                          </mark>
                        ),
                      )}
                    </p>
                  ))}
                </div>
              </section>
            ))}
          </article>
        </div>
      )}
    </div>
  );
}
