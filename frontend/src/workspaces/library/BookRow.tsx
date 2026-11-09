import { useState } from "react";
import type { ApiClient, Book, Collection } from "../../api/client";

const STATUS_LABEL: Record<string, string> = {
  queued: "Queued",
  extracting: "Reading",
  ocr: "Scanning",
  structuring: "Structuring",
  indexing: "Indexing",
  ready: "Ready",
  failed: "Failed",
};

interface Props {
  client: ApiClient;
  book: Book;
  collections: Collection[];
  onOpen: () => void;
  onChanged: () => void;
  onError: (reason: unknown) => void;
}

export function BookRow({ client, book, collections, onOpen, onChanged, onError }: Props) {
  const [confirming, setConfirming] = useState(false);
  const processing = book.status !== "ready" && book.status !== "failed";

  async function remove() {
    try {
      await client.deleteBook(book.id);
      onChanged();
    } catch (reason) {
      onError(reason);
    }
  }

  async function toggleCollection(id: number) {
    const ids = book.collection_ids.includes(id)
      ? book.collection_ids.filter((value) => value !== id)
      : [...book.collection_ids, id];
    try {
      await client.setBookCollections(book.id, ids);
      onChanged();
    } catch (reason) {
      onError(reason);
    }
  }

  return (
    <li className="py-4" data-testid="book-row">
      <div className="flex items-start justify-between gap-6">
        <div className="min-w-0">
          {book.status === "ready" ? (
            <button type="button" onClick={onOpen} className="text-left font-serif text-lg hover:text-accent">
              {book.title}
            </button>
          ) : (
            <p className="font-serif text-lg">{book.title}</p>
          )}
          <p className="mt-0.5 text-xs text-muted">
            {[
              book.author,
              book.page_count ? `${book.page_count} pages` : null,
              book.chapter_count ? `${book.chapter_count} chapters` : null,
              book.needs_ocr ? "Scanned (OCR)" : null,
              book.file_name,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          {collections.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {collections.map((collection) => {
                const member = book.collection_ids.includes(collection.id);
                return (
                  <button
                    key={collection.id}
                    type="button"
                    aria-pressed={member}
                    onClick={() => void toggleCollection(collection.id)}
                    className={`rounded-full border px-2 py-0.5 text-xs ${
                      member ? "border-accent bg-accent-soft text-accent" : "border-rule text-muted hover:text-ink"
                    }`}
                  >
                    {collection.name}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-4 text-xs">
          <span data-testid="book-status" className={book.status === "failed" ? "text-danger" : "text-muted"}>
            {STATUS_LABEL[book.status] ?? book.status}
          </span>
          {confirming ? (
            <span className="flex gap-2">
              <button type="button" onClick={() => void remove()} className="text-danger">
                Confirm remove
              </button>
              <button type="button" onClick={() => setConfirming(false)} className="text-muted">
                Keep
              </button>
            </span>
          ) : (
            <button type="button" onClick={() => setConfirming(true)} className="text-muted hover:text-danger">
              Remove
            </button>
          )}
        </div>
      </div>

      {processing && (
        <div className="mt-3">
          <div
            role="progressbar"
            aria-valuenow={Math.round(book.progress * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
            className="h-1 overflow-hidden rounded-full bg-rule"
          >
            <div className="h-full bg-accent transition-all" style={{ width: `${book.progress * 100}%` }} />
          </div>
          {book.status_detail && <p className="mt-1 text-xs text-muted">{book.status_detail}</p>}
        </div>
      )}
      {!processing && book.status_detail && (
        <p className={`mt-2 text-xs ${book.status === "failed" ? "text-danger" : "text-muted"}`}>
          {book.status_detail}
        </p>
      )}
    </li>
  );
}
