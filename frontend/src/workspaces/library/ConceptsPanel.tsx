import { useCallback, useEffect, useState } from "react";
import type { ApiClient, Chapter, ConceptView } from "../../api/client";

interface Props {
  client: ApiClient;
  bookId: number;
  chapters: Chapter[];
}

export function ConceptsPanel({ client, bookId, chapters }: Props) {
  const [concepts, setConcepts] = useState<ConceptView[]>([]);
  const [filter, setFilter] = useState("");
  const [chapterId, setChapterId] = useState<number | null>(chapters[0]?.id ?? null);
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(
    () => client.bookConcepts(bookId).then(setConcepts, (reason) => setMessage(String(reason))),
    [client, bookId],
  );

  useEffect(() => {
    void load();
  }, [load]);

  async function enrich() {
    if (chapterId === null) return;
    setWorking(true);
    setMessage(null);
    try {
      const { added } = await client.enrichConcepts(chapterId);
      setMessage(added === 0 ? "No new concepts were found." : `Added ${added} concepts, each checked against the text.`);
      await load();
    } catch (reason) {
      setMessage(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setWorking(false);
    }
  }

  const needle = filter.trim().toLowerCase();
  const visible = needle ? concepts.filter((concept) => concept.name.toLowerCase().includes(needle)) : concepts;

  return (
    <section aria-label="Concepts" className="max-w-3xl">
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <input
          type="search"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter concepts"
          aria-label="Filter concepts"
          className="w-56 rounded-md border border-rule bg-surface px-3 py-1.5 text-sm"
        />
        <select
          aria-label="Chapter to analyse"
          value={chapterId ?? ""}
          onChange={(event) => setChapterId(Number(event.target.value))}
          className="max-w-64 rounded-md border border-rule bg-surface px-2 py-1.5 text-sm"
        >
          {chapters.map((chapter) => (
            <option key={chapter.id} value={chapter.id}>
              {chapter.title}
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={working || chapterId === null}
          onClick={() => void enrich()}
          className="rounded-md border border-rule px-3 py-1.5 text-sm hover:text-accent disabled:opacity-50"
        >
          {working ? "Analysing…" : "Find more with AI"}
        </button>
      </div>

      {message && <p role="status" className="mb-4 text-xs text-muted">{message}</p>}
      <p className="mb-3 text-xs text-muted">{visible.length} concepts</p>

      <ul className="divide-y divide-rule border-y border-rule">
        {visible.map((concept) => (
          <li key={concept.id} data-testid="concept" className="py-3">
            <div className="flex items-baseline justify-between gap-4">
              <h3 className="font-serif text-base">{concept.name}</h3>
              {concept.pages.length > 0 && (
                <span className="shrink-0 text-xs text-muted">p. {concept.pages.slice(0, 4).join(", ")}</span>
              )}
            </div>
            {concept.definition && <p className="mt-1 text-sm leading-relaxed">{concept.definition}</p>}
            {concept.related.length > 0 && (
              <p className="mt-1 text-xs text-muted">Related: {concept.related.join(", ")}</p>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
