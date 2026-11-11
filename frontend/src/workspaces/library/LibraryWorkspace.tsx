import { type ChangeEvent, type DragEvent, type FormEvent, useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient, Book, Collection } from "../../api/client";
import { BookDetail, type OpenTarget } from "./BookDetail";
import { BookRow } from "./BookRow";
import { SearchBox, SearchResults, useSearch } from "./Search";

const EVERYWHERE = { kind: "all" } as const;

const POLL_MS = 1200;

export function isProcessing(book: Book) {
  return book.status !== "ready" && book.status !== "failed";
}

export function LibraryWorkspace({ client }: { client: ApiClient }) {
  const [books, setBooks] = useState<Book[]>([]);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [query, setQuery] = useState("");
  const [collectionId, setCollectionId] = useState<number | null>(null);
  const [newCollection, setNewCollection] = useState("");
  const [openBookId, setOpenBookId] = useState<number | null>(null);
  /** The search result the open book should show. `visit` remounts the reader for each new result. */
  const [target, setTarget] = useState<(OpenTarget & { visit: number }) | null>(null);
  const search = useSearch(client, EVERYWHERE);
  const [message, setMessage] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [uploading, setUploading] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  const report = useCallback((reason: unknown) => {
    setMessage(reason instanceof Error ? reason.message : String(reason));
  }, []);

  const refresh = useCallback(async () => {
    const [nextBooks, nextCollections] = await Promise.all([
      client.listBooks(query, collectionId),
      client.listCollections(),
    ]);
    setBooks(nextBooks);
    setCollections(nextCollections);
  }, [client, query, collectionId]);

  useEffect(() => {
    refresh().catch(report);
  }, [refresh, report]);

  const anyProcessing = books.some(isProcessing);
  useEffect(() => {
    if (!anyProcessing) return;
    const timer = setInterval(() => refresh().catch(report), POLL_MS);
    return () => clearInterval(timer);
  }, [anyProcessing, refresh, report]);

  async function upload(files: Iterable<File>) {
    setMessage(null);
    for (const file of files) {
      setUploading((count) => count + 1);
      try {
        await client.uploadBook(file);
      } catch (reason) {
        report(reason);
      } finally {
        setUploading((count) => count - 1);
      }
      await refresh().catch(report);
    }
  }

  function onPick(event: ChangeEvent<HTMLInputElement>) {
    if (event.target.files) void upload(Array.from(event.target.files));
    event.target.value = "";
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    setDragging(false);
    void upload(Array.from(event.dataTransfer.files));
  }

  async function createCollection(event: FormEvent) {
    event.preventDefault();
    try {
      await client.createCollection(newCollection);
      setNewCollection("");
      await refresh();
    } catch (reason) {
      report(reason);
    }
  }

  async function removeCollection(id: number) {
    try {
      await client.deleteCollection(id);
      if (collectionId === id) setCollectionId(null);
      await refresh();
    } catch (reason) {
      report(reason);
    }
  }

  function openResult(bookId: number, next: OpenTarget) {
    // The book may be hidden by the title or collection filter, so show everything.
    if (!books.some((book) => book.id === bookId)) {
      setQuery("");
      setCollectionId(null);
    }
    setTarget((previous) => ({ ...next, visit: (previous?.visit ?? 0) + 1 }));
    setOpenBookId(bookId);
  }

  function openFromLibrary(bookId: number, sectionId: number, passageId: number, title: string) {
    const text = search.query.trim();
    client.rememberSearch(text, EVERYWHERE, title).catch(() => undefined);
    openResult(bookId, { sectionId, passageId, query: text });
  }

  const openBook = books.find((book) => book.id === openBookId);
  if (openBook) {
    return (
      <BookDetail
        key={`${openBook.id}:${target?.visit ?? 0}`}
        client={client}
        book={openBook}
        initial={target ?? undefined}
        onOpenBook={openResult}
        onBack={() => {
          setOpenBookId(null);
          setTarget(null);
        }}
      />
    );
  }
  const searching = search.query.trim() !== "";

  return (
    <div
      className={`-m-3 rounded-lg p-3 ${dragging ? "bg-accent-soft" : ""}`}
      onDragOver={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      <div className="mb-5">
        <SearchBox
          client={client}
          label="Search all books"
          query={search.query}
          onQuery={search.setQuery}
          onCommit={() => void client.rememberSearch(search.query.trim(), EVERYWHERE, null).catch(() => undefined)}
        />
      </div>

      {search.error && (
        <p role="alert" className="mb-4 rounded-md border border-danger/30 p-3 text-sm text-danger">
          {search.error}
        </p>
      )}

      {searching && (
        <div className="max-w-3xl">
          <SearchResults
            result={search.result}
            pending={search.pending}
            onOpen={(hit) => openFromLibrary(hit.book_id, hit.section_id, hit.passage_id, hit.section_title)}
            onOpenConcept={(concept) => openFromLibrary(concept.book_id, concept.section_id, concept.passage_id, concept.name)}
          />
        </div>
      )}

      <div hidden={searching}>
      <div className="mb-6 flex items-center gap-3">
        <input
          ref={input}
          type="file"
          accept="application/pdf,.pdf"
          multiple
          aria-label="Choose PDF files"
          className="hidden"
          onChange={onPick}
        />
        <button
          type="button"
          onClick={() => input.current?.click()}
          className="rounded-md bg-accent px-4 py-2 text-sm text-white hover:opacity-90"
        >
          Add PDF
        </button>
        <input
          type="search"
          value={query}
          placeholder="Filter by title"
          aria-label="Filter by title"
          onChange={(event) => setQuery(event.target.value)}
          className="w-64 rounded-md border border-rule bg-surface px-3 py-2 text-sm"
        />
        {uploading > 0 && <span className="text-sm text-muted">Uploading…</span>}
      </div>

      {message && (
        <p role="alert" className="mb-4 rounded-md border border-danger/30 p-3 text-sm text-danger">
          {message}
        </p>
      )}

      <div className="grid grid-cols-[12rem_minmax(0,1fr)] gap-8">
        <aside aria-label="Collections">
          <h2 className="mb-2 text-xs uppercase tracking-wide text-muted">Collections</h2>
          <ul className="mb-3 space-y-0.5 text-sm">
            <li>
              <button
                type="button"
                aria-current={collectionId === null ? "true" : undefined}
                onClick={() => setCollectionId(null)}
                className={collectionId === null ? "font-medium text-accent" : "hover:text-accent"}
              >
                All documents
              </button>
            </li>
            {collections.map((collection) => (
              <li key={collection.id} className="group flex items-center justify-between gap-2">
                <button
                  type="button"
                  aria-current={collectionId === collection.id ? "true" : undefined}
                  onClick={() => setCollectionId(collection.id)}
                  className={`truncate text-left ${collectionId === collection.id ? "font-medium text-accent" : "hover:text-accent"}`}
                >
                  {collection.name} <span className="text-muted">{collection.book_count}</span>
                </button>
                <button
                  type="button"
                  aria-label={`Delete collection ${collection.name}`}
                  onClick={() => void removeCollection(collection.id)}
                  className="text-xs text-muted opacity-0 hover:text-danger focus:opacity-100 group-hover:opacity-100"
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
          <form onSubmit={createCollection}>
            <input
              value={newCollection}
              onChange={(event) => setNewCollection(event.target.value)}
              placeholder="New collection"
              aria-label="New collection name"
              className="w-full rounded-md border border-rule bg-surface px-2 py-1 text-sm"
            />
          </form>
        </aside>

        <section aria-label="Documents">
          {books.length === 0 ? (
            <p className="text-sm text-muted">
              {query || collectionId !== null ? "No documents match." : "Add a PDF or drop one here to begin."}
            </p>
          ) : (
            <ul className="divide-y divide-rule border-y border-rule">
              {books.map((book) => (
                <BookRow
                  key={book.id}
                  client={client}
                  book={book}
                  collections={collections}
                  onOpen={() => setOpenBookId(book.id)}
                  onChanged={() => refresh().catch(report)}
                  onError={report}
                />
              ))}
            </ul>
          )}
        </section>
      </div>
      </div>
    </div>
  );
}
