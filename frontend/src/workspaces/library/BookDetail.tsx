import { type FormEvent, useEffect, useRef, useState } from "react";
import type { ApiClient, Book, Chapter, Citation, Passage, SearchHit, Section } from "../../api/client";
import { AskPanel } from "./AskPanel";
import { ConceptsPanel } from "./ConceptsPanel";

function pages(start: number, end: number) {
  return start === end ? `p. ${start}` : `pp. ${start}–${end}`;
}

/** Renders the backend's [[match]] markers without treating document text as HTML. */
function Snippet({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\[\[.*?\]\])/).map((part, index) =>
        part.startsWith("[[") ? (
          <mark key={index} className="bg-accent-soft text-accent">
            {part.slice(2, -2)}
          </mark>
        ) : (
          <span key={index}>{part}</span>
        ),
      )}
    </>
  );
}

interface Props {
  client: ApiClient;
  book: Book;
  onBack: () => void;
}

export function BookDetail({ client, book, onBack }: Props) {
  const [chapters, setChapters] = useState<Chapter[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [section, setSection] = useState<Section | null>(null);
  const [passages, setPassages] = useState<Passage[]>([]);
  const [focusPassage, setFocusPassage] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"read" | "ask" | "concepts">("read");
  const focused = useRef<HTMLElement>(null);

  useEffect(() => {
    client.bookStructure(book.id).then((result) => {
      setChapters(result);
      const first = result.find((chapter) => chapter.sections.some((item) => item.passage_count > 0));
      if (first) {
        setExpanded(first.id);
        setSection(first.sections.find((item) => item.passage_count > 0) ?? null);
      }
    }, (reason) => setError(String(reason)));
  }, [client, book.id]);

  useEffect(() => {
    if (!section) return;
    client.sectionPassages(section.id).then(setPassages, (reason) => setError(String(reason)));
  }, [client, section]);

  useEffect(() => {
    focused.current?.scrollIntoView({ block: "center" });
  }, [passages, focusPassage]);

  async function search(event: FormEvent) {
    event.preventDefault();
    if (!query.trim()) {
      setHits(null);
      return;
    }
    try {
      setHits(await client.searchBook(book.id, query));
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  function openHit(hit: SearchHit) {
    const chapter = chapters.find((item) => item.id === hit.chapter_id);
    const target = chapter?.sections.find((item) => item.id === hit.section_id);
    if (!target) return;
    setExpanded(hit.chapter_id);
    setFocusPassage(hit.passage_id);
    setSection(target);
    setHits(null);
  }

  function openCitation(citation: Citation) {
    const chapter = chapters.find((item) => item.sections.some((entry) => entry.id === citation.section_id));
    const target = chapter?.sections.find((entry) => entry.id === citation.section_id);
    if (!chapter || !target) return;
    setExpanded(chapter.id);
    setFocusPassage(citation.passage_id);
    setSection(target);
    setHits(null);
    setView("read");
  }

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
        {(["read", "ask", "concepts"] as const).map((item) => (
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
        <form onSubmit={search} className="my-5">
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search inside this book"
            aria-label="Search inside this book"
            className="w-96 max-w-full rounded-md border border-rule bg-surface px-3 py-2 text-sm"
          />
        </form>
      )}

      {error && <p role="alert" className="mb-4 text-sm text-danger">{error}</p>}

      {/* Kept mounted so a running answer survives switching to the reader to check a citation. */}
      <div hidden={view !== "ask"}>
        <AskPanel client={client} bookId={book.id} onOpenCitation={openCitation} />
      </div>

      {view === "concepts" && <ConceptsPanel client={client} bookId={book.id} chapters={chapters} />}

      {view !== "read" ? null : hits !== null ? (
        <section aria-label="Search results">
          <p className="mb-3 text-xs text-muted">{hits.length} results</p>
          <ul className="divide-y divide-rule border-y border-rule">
            {hits.map((hit) => (
              <li key={hit.passage_id}>
                <button type="button" onClick={() => openHit(hit)} className="block w-full py-3 text-left">
                  <span className="text-xs text-muted">
                    {hit.chapter_title} · {hit.section_title} · {pages(hit.start_page, hit.end_page)}
                  </span>
                  <span className="mt-1 block font-serif text-sm leading-relaxed">
                    <Snippet text={hit.snippet} />
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : (
        <div className="grid grid-cols-[18rem_minmax(0,1fr)] gap-10">
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

          <article aria-label="Passages" className="max-w-2xl">
            {section && <h3 className="mb-4 font-serif text-xl">{section.title}</h3>}
            {passages.map((passage) => (
              <section
                key={passage.id}
                ref={passage.id === focusPassage ? focused : undefined}
                data-testid="passage"
                className={`mb-6 ${passage.id === focusPassage ? "rounded-md bg-accent-soft p-3" : ""}`}
              >
                <p className="mb-1 text-xs text-muted">{pages(passage.start_page, passage.end_page)}</p>
                <div className="space-y-2 font-serif leading-relaxed">
                  {passage.text.split("\n\n").map((paragraph, index) => (
                    <p key={index}>{paragraph}</p>
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
