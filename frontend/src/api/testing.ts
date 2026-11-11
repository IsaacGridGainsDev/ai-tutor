import type { Invoker } from "./client";

type Routes = (url: string, init?: RequestInit) => Promise<Response>;

/**
 * Lets tests describe the desktop app's engine commands as the HTTP routes they replaced, so one
 * route table covers both the commands and the calls that still go to the backend.
 */
export function invokerOver(routes: Routes, base = "http://x/api"): Invoker {
  // Requests the page asked to stop, as the engine remembers them by id.
  const stops = new Map<string, AbortController>();
  return async (command, args = {}) => {
    if (command === "stop_request") {
      stops.get(String(args.id))?.abort();
      return undefined;
    }
    if (command === "ask" || command === "model_pull") {
      // These report events. The route they replaced sent the same events one per line.
      const controller = new AbortController();
      stops.set(String(args.id), controller);
      const [path, body] = command === "ask" ? [`/books/${args.bookId}/ask`, { question: args.question }] : ["/ai/models/pull", { name: args.name }];
      const onEvent = args.onEvent as (event: unknown) => void;
      try {
        const response = await routes(`${base}${path}`, { method: "POST", body: JSON.stringify(body), signal: controller.signal });
        if (!response.ok || !response.body) throw (await response.json().catch(() => null))?.detail ?? response.statusText;
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffered = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buffered += decoder.decode(value, { stream: true });
          const lines = buffered.split("\n");
          buffered = lines.pop() ?? "";
          for (const line of lines) if (line) onEvent(JSON.parse(line));
        }
      } catch (reason) {
        if (!controller.signal.aborted) throw reason;
      }
      return undefined;
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const given = args as Record<string, any>;
    const { bookId, chapterId, sectionId, id, draft, text, q, collectionId, collectionIds, name, scope, live, query, resultTitle, settings } = given;
    const within = new URLSearchParams();
    if (sectionId != null) within.set("section_id", String(sectionId));
    else if (chapterId != null) within.set("chapter_id", String(chapterId));
    const json = (body: unknown, method = "POST"): RequestInit => ({ method, body: JSON.stringify(body) });
    const params = (values: Record<string, unknown>) => {
      const kept = Object.entries(values).filter(([, value]) => value != null).map(([key, value]) => [key, String(value)]);
      return kept.length ? `?${new URLSearchParams(kept)}` : "";
    };
    const [path, init]: [string, RequestInit?] =
      command === "book_visuals" ? [`/books/${bookId}/visuals?${within}`]
      : command === "diagram_delete" ? [`/diagrams/${id}`, { method: "DELETE" }]
      : command === "climate_list" ? [`/books/${bookId}/climate`]
      : command === "climate_candidates" ? [`/books/${bookId}/climate/candidates`]
      : command === "climate_save" ? (id == null ? [`/books/${bookId}/climate`, json(draft)] : [`/climate/${id}`, json(draft, "PUT")])
      : command === "climate_delete" ? [`/climate/${id}`, { method: "DELETE" }]
      : command === "climate_parse" ? ["/climate/parse", json({ text })]
      : command === "books_list" ? [`/books${params({ q, collection_id: collectionId })}`]
      : command === "book_set_collections" ? [`/books/${id}/collections`, json({ collection_ids: collectionIds }, "PUT")]
      : command === "book_structure" ? [`/books/${id}/structure`]
      : command === "section_passages" ? [`/sections/${id}/passages${params({ q })}`]
      : command === "collections_list" ? ["/collections"]
      : command === "collection_create" ? ["/collections", json({ name })]
      : command === "collection_delete" ? [`/collections/${id}`, { method: "DELETE" }]
      : command === "search_library" ? [`/search${params({ q, scope: scope.kind, book_id: scope.book_id, chapter_id: scope.chapter_id, section_id: scope.section_id, live })}`]
      : command === "search_recent" ? ["/search/recent"]
      : command === "search_remember" ? ["/search/recent", json({ query, scope, book_id: bookId, result_title: resultTitle })]
      : command === "search_clear_recent" ? ["/search/recent", { method: "DELETE" }]
      : command === "settings_get" ? ["/settings"]
      : command === "settings_put" ? ["/settings", json(settings, "PUT")]
      : command === "ai_status" ? ["/ai/status"]
      : command === "model_delete" ? [`/ai/models${params({ name })}`, { method: "DELETE" }]
      : command === "book_concepts" ? [`/books/${id}/concepts`]
      : command === "concepts_enrich" ? [`/chapters/${chapterId}/concepts/enrich`, { method: "POST" }]
      : command === "section_draw" ? [`/sections/${sectionId}/diagram`, { method: "POST" }]
      : [`/unknown-command/${command}`];
    const response = await routes(`${base}${path}`, init);
    if (!response.ok) throw (await response.json().catch(() => null))?.detail ?? response.statusText;
    return response.status === 204 ? undefined : response.json();
  };
}
