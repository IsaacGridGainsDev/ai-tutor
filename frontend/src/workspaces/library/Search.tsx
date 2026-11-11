import { type FormEvent, type Ref, useEffect, useState } from "react";
import {
  type ApiClient,
  type ConceptMatch,
  MARK_END,
  MARK_START,
  type RecentSearch,
  type ScopeKind,
  type SearchHit,
  type SearchResponse,
  type SearchScope,
} from "../../api/client";

const DEBOUNCE_MS = 150;
const PIN_KEY = "tutor.search.pinned";
const MARKS = new RegExp(`${MARK_START}([^${MARK_END}]*)${MARK_END}`, "g");

export function pages(start: number, end: number) {
  return start === end ? `p. ${start}` : `pp. ${start}–${end}`;
}

export interface Segment {
  text: string;
  marked: boolean;
}

/** Splits text on the backend's match markers. Document text is never treated as HTML. */
export function splitMarks(text: string): Segment[] {
  const segments: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(MARKS)) {
    if (match.index > last) segments.push({ text: text.slice(last, match.index), marked: false });
    segments.push({ text: match[1], marked: true });
    last = match.index + match[0].length;
  }
  if (last < text.length) segments.push({ text: text.slice(last), marked: false });
  return segments;
}

function Snippet({ text }: { text: string }) {
  return (
    <>
      {splitMarks(text).map((segment, index) =>
        segment.marked ? (
          <mark key={index} className="bg-accent-soft text-accent">
            {segment.text}
          </mark>
        ) : (
          <span key={index}>{segment.text}</span>
        ),
      )}
    </>
  );
}

/** Live search: runs shortly after typing stops and cancels the request it replaces. */
export function useSearch(client: ApiClient, scope: SearchScope, initialQuery = "") {
  const [query, setQuery] = useState(initialQuery);
  const [result, setResult] = useState<SearchResponse | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scopeKey = JSON.stringify(scope);

  useEffect(() => {
    if (!query.trim()) {
      setResult(null);
      setPending(false);
      setError(null);
      return;
    }
    const controller = new AbortController();
    setPending(true);
    const timer = setTimeout(() => {
      // A trailing space is kept: it tells the engine the last word is finished, not a prefix.
      client.search(query.trimStart(), JSON.parse(scopeKey) as SearchScope).then(
        (next) => {
          if (controller.signal.aborted) return;
          setResult(next);
          setError(null);
          setPending(false);
        },
        (reason) => {
          if (controller.signal.aborted) return;
          setError(reason instanceof Error ? reason.message : String(reason));
          setPending(false);
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [client, query, scopeKey]);

  return { query, setQuery, result, pending, error };
}

/** Whether the results pane stays open beside the text. Remembered on this computer. */
export function usePinned(): [boolean, (next: boolean) => void] {
  const [pinned, setPinned] = useState(() => {
    try {
      return localStorage.getItem(PIN_KEY) === "1";
    } catch {
      return false;
    }
  });
  return [
    pinned,
    (next) => {
      setPinned(next);
      try {
        localStorage.setItem(PIN_KEY, next ? "1" : "0");
      } catch {
        // The preference simply does not persist.
      }
    },
  ];
}

export interface ScopeOption {
  kind: ScopeKind;
  label: string;
}

interface BoxProps {
  client: ApiClient;
  label: string;
  query: string;
  onQuery: (query: string) => void;
  /** The learner pressed Enter: the query is worth remembering. */
  onCommit: () => void;
  scopes?: ScopeOption[];
  scope?: ScopeKind;
  onScope?: (kind: ScopeKind) => void;
  pinned?: boolean;
  onPin?: (pinned: boolean) => void;
  inputRef?: Ref<HTMLInputElement>;
}

export function SearchBox({ client, label, query, onQuery, onCommit, scopes, scope, onScope, pinned, onPin, inputRef }: BoxProps) {
  const [focused, setFocused] = useState(false);
  const [recent, setRecent] = useState<RecentSearch[]>([]);

  function loadRecent() {
    client.recentSearches().then(setRecent, () => setRecent([]));
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    if (query.trim()) onCommit();
  }

  function pick(item: RecentSearch) {
    if (scopes?.some((option) => option.kind === item.scope)) onScope?.(item.scope);
    onQuery(item.query);
  }

  function clearRecent() {
    client.clearRecentSearches().then(() => setRecent([]), () => undefined);
  }

  const showRecent = focused && !query.trim() && recent.length > 0;

  return (
    <form
      onSubmit={submit}
      role="search"
      className="relative flex flex-wrap items-center gap-2"
      onFocus={() => {
        if (!focused) loadRecent();
        setFocused(true);
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
      }}
    >
      <input
        ref={inputRef}
        type="search"
        value={query}
        onChange={(event) => onQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && query) {
            event.preventDefault();
            onQuery("");
          }
        }}
        placeholder={label}
        aria-label={label}
        className="w-96 max-w-full rounded-md border border-rule bg-surface px-3 py-2 text-sm"
      />
      {scopes && scopes.length > 1 && (
        <select
          aria-label="Search scope"
          value={scope}
          onChange={(event) => onScope?.(event.target.value as ScopeKind)}
          className="rounded-md border border-rule bg-surface px-2 py-2 text-sm"
        >
          {scopes.map((option) => (
            <option key={option.kind} value={option.kind}>
              {option.label}
            </option>
          ))}
        </select>
      )}
      {onPin && (
        <button
          type="button"
          aria-pressed={pinned}
          onClick={() => onPin(!pinned)}
          title="Keep the results open beside the text"
          className={`rounded-md border px-2 py-2 text-xs ${
            pinned ? "border-accent bg-accent-soft text-accent" : "border-rule text-muted hover:text-ink"
          }`}
        >
          {pinned ? "Results pinned" : "Pin results"}
        </button>
      )}

      {showRecent && (
        <div
          aria-label="Recent searches"
          role="group"
          className="absolute left-0 top-full z-10 mt-1 w-96 max-w-full rounded-md border border-rule bg-surface py-1 text-sm shadow-lg"
        >
          <div className="flex items-center justify-between px-3 py-1 text-xs text-muted">
            <span>Recent searches</span>
            <button type="button" onClick={clearRecent} className="hover:text-danger">
              Clear
            </button>
          </div>
          <ul>
            {recent.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  onClick={() => pick(item)}
                  className="flex w-full items-baseline justify-between gap-3 px-3 py-1.5 text-left hover:bg-rule/50"
                >
                  <span className="truncate">{item.query}</span>
                  <span className="shrink-0 truncate text-xs text-muted">
                    {[item.result_title, item.book_title].filter(Boolean).join(" · ")}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </form>
  );
}

interface ResultsProps {
  result: SearchResponse | null;
  pending: boolean;
  /** Shown beside the text: tighter, with no column constraints of its own. */
  compact?: boolean;
  activePassageId?: number | null;
  onOpen: (hit: SearchHit) => void;
  onOpenConcept: (concept: ConceptMatch) => void;
}

export function SearchResults({ result, pending, compact, activePassageId, onOpen, onOpenConcept }: ResultsProps) {
  if (!result) {
    return (
      <section aria-label="Search results">
        <p className="text-xs text-muted">{pending ? "Searching…" : ""}</p>
      </section>
    );
  }
  const several = result.groups.length > 1;
  const count = `${result.total} ${result.total === 1 ? "result" : "results"}`;

  return (
    <section aria-label="Search results" aria-busy={pending} className={pending ? "opacity-60" : ""}>
      <p role="status" className="mb-3 text-xs text-muted">
        {result.total === 0 && result.concepts.length === 0 ? "No results. Check the spelling, or widen the scope." : count}
        {several && ` in ${result.groups.length} books`}
        {result.match_mode === "any" && result.total > 0 && " · no passage has every word, so these have any of them"}
      </p>

      {result.concepts.length > 0 && (
        <div aria-label="Matching concepts" role="group" className="mb-4">
          <h3 className="mb-1.5 text-xs uppercase tracking-wide text-muted">Concepts</h3>
          <ul className="flex flex-wrap gap-1.5">
            {result.concepts.map((concept) => (
              <li key={concept.id}>
                <button
                  type="button"
                  title={concept.definition ?? undefined}
                  onClick={() => onOpenConcept(concept)}
                  className="rounded-full border border-rule bg-surface px-2.5 py-1 text-xs hover:border-accent hover:text-accent"
                >
                  {concept.name}
                  <span className="text-muted">
                    {" "}
                    · {several || compact ? `${concept.book_title}, ` : ""}p. {concept.start_page}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {result.groups.map((group) => (
        <div key={group.book_id} aria-label={group.book_title} role="group" className="mb-5">
          <h3 className="mb-1 flex items-baseline justify-between gap-3 border-b border-rule pb-1 text-sm font-medium">
            <span className="truncate">{group.book_title}</span>
            <span className="shrink-0 text-xs font-normal text-muted">
              {group.hits.length < group.total ? `top ${group.hits.length} of ${group.total}` : group.total}
            </span>
          </h3>
          <ul className="divide-y divide-rule">
            {group.hits.map((hit) => (
              <li key={hit.passage_id}>
                <button
                  type="button"
                  aria-current={hit.passage_id === activePassageId ? "true" : undefined}
                  onClick={() => onOpen(hit)}
                  className={`block w-full rounded-sm px-1 text-left hover:bg-rule/40 ${compact ? "py-2" : "py-3"} ${
                    hit.passage_id === activePassageId ? "bg-accent-soft" : ""
                  }`}
                >
                  <span className="text-xs text-muted">
                    {hit.chapter_title === hit.section_title ? "" : `${hit.chapter_title} › `}
                    {hit.section_title} · {pages(hit.start_page, hit.end_page)}
                    {hit.from_ocr && " · scanned"}
                  </span>
                  <span className={`mt-1 block font-serif leading-relaxed ${compact ? "text-xs" : "text-sm"}`}>
                    <Snippet text={hit.snippet} />
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </section>
  );
}
