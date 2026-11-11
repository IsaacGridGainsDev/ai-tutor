import { Channel, invoke, isTauri } from "@tauri-apps/api/core";

export interface BackendInfo {
  baseUrl: string;
  token: string | null;
}

export interface Health {
  status: string;
  version: string;
  schema_version: number;
}

export interface AppSettings {
  llm_model: string | null;
  embedding_model: string | null;
  speech_model: string | null;
  voice: string | null;
  reading_speed: number;
  explanation_depth: "brief" | "standard" | "deep";
  assessment_difficulty: "gentle" | "standard" | "rigorous";
  compute_preference: "auto" | "gpu" | "cpu";
  document_storage_path: string | null;
}

export interface Book {
  id: number;
  title: string;
  author: string | null;
  file_name: string;
  page_count: number;
  status: string;
  status_detail: string | null;
  progress: number;
  needs_ocr: boolean;
  created_at: string;
  chapter_count: number;
  passage_count: number;
  collection_ids: number[];
}

export interface Collection {
  id: number;
  name: string;
  book_count: number;
}

export interface Section {
  id: number;
  title: string;
  start_page: number;
  end_page: number;
  passage_count: number;
}

export interface Chapter {
  id: number;
  title: string;
  start_page: number;
  end_page: number;
  detection_method: string;
  sections: Section[];
}

export interface Passage {
  id: number;
  section_id: number;
  ordinal: number;
  text: string;
  start_page: number;
  end_page: number;
  /** The text with matches wrapped in MARK_START and MARK_END, when a query was sent and matched. */
  highlighted: string | null;
}

/** The engine marks matches with control characters, which cannot occur in a book's text. */
export const MARK_START = "";
export const MARK_END = "";

export type ScopeKind = "all" | "book" | "chapter" | "section";

export interface SearchScope {
  kind: ScopeKind;
  book_id?: number;
  chapter_id?: number;
  section_id?: number;
}

export interface SearchHit {
  passage_id: number;
  book_id: number;
  chapter_id: number;
  section_id: number;
  chapter_title: string;
  section_title: string;
  snippet: string;
  start_page: number;
  end_page: number;
  from_ocr: boolean;
}

export interface SearchGroup {
  book_id: number;
  book_title: string;
  total: number;
  hits: SearchHit[];
}

export interface ConceptMatch {
  id: number;
  name: string;
  definition: string | null;
  book_id: number;
  book_title: string;
  passage_id: number;
  section_id: number;
  start_page: number;
}

export interface SearchResponse {
  query: string;
  match_mode: "all" | "any" | null;
  total: number;
  groups: SearchGroup[];
  concepts: ConceptMatch[];
}

export interface RecentSearch {
  id: number;
  query: string;
  scope: ScopeKind;
  book_id: number | null;
  book_title: string | null;
  result_title: string | null;
  created_at: string;
}

export interface ModelView {
  name: string;
  size_bytes: number;
  family: string | null;
  parameter_size: string | null;
  quantization: string | null;
  can_chat: boolean;
  can_embed: boolean;
}

export interface AiStatus {
  reachable: boolean;
  version: string | null;
  message: string | null;
  active_model: string | null;
  models: ModelView[];
  excluded_cloud_models: string[];
  running: { name: string; processor: "cpu" | "gpu" | "mixed"; vram_bytes: number; size_bytes: number }[];
  embedding: { mode: "model" | "hashing"; model: string | null };
}

export interface Citation {
  passage_id: number;
  section_id: number;
  chapter_title: string;
  section_title: string;
  start_page: number;
  end_page: number;
}

export interface AnswerSegment {
  kind: "supported" | "background" | "interpretation" | "not_found";
  text: string;
  quote: string | null;
  citation: Citation | null;
  downgraded: boolean;
}

export type AskEvent =
  | { type: "stage"; stage: "retrieving" | "generating" }
  | { type: "progress"; tokens: number }
  | {
      type: "answer";
      found_in_document: boolean;
      segments: AnswerSegment[];
      concepts: string[];
      model: string | null;
      stats?: { total_seconds: number; completion_tokens: number };
    }
  | { type: "error"; message: string };

export type PullEvent =
  | { type: "progress"; status: string; total: number; completed: number }
  | { type: "done" }
  | { type: "error"; message: string };

export interface MonthTemperature {
  /** 1 for January to 12 for December. */
  month: number;
  max: number;
  min: number;
}

export interface Wind {
  /** One of sixteen compass points, such as "SW" or "NNE": where the wind blows from. */
  direction: string;
  /** Percentage of the time. */
  share: number;
}

export interface ClimateTables {
  months: MonthTemperature[];
  winds: Wind[];
}

export interface ClimateOrigin {
  passage_id: number;
  section_id: number;
  section_title: string;
  start_page: number;
  end_page: number;
}

export type ClimateCandidate = ClimateOrigin & ClimateTables;

export type FigureKind = "wind_rose" | "comfort_chart" | "section" | "model_3d";

export interface FigureOffer {
  kind: FigureKind;
  title: string;
  available: boolean;
  /** What data the figure needs, shown when it is unavailable. */
  needs: string;
  /** What in the figure is data and what is schematic. */
  basis: string;
}

export interface ClimateDraft extends ClimateTables {
  name: string;
  source: "entered" | "book";
  passage_id: number | null;
}

export interface ClimateDataset extends ClimateTables {
  id: number;
  book_id: number;
  name: string;
  source: "entered" | "book";
  origin: ClimateOrigin | null;
  summary: {
    prevailing_wind: Wind | null;
    calm_share: number | null;
    hottest: { month: number; max: number } | null;
    coolest: { month: number; min: number } | null;
    mean_daily_range: number | null;
  };
  figures: FigureOffer[];
  created_at: string;
}

/** Where in the book something was found. */
export interface VisualOrigin {
  passage_id: number;
  section_id: number;
  section_title: string;
  start_page: number;
  end_page: number;
}

export interface ConceptNode extends VisualOrigin {
  id: number;
  name: string;
  definition: string | null;
  importance: number;
  /** Passages in scope that mention it. */
  mentions: number;
}

export interface ConceptMapData {
  nodes: ConceptNode[];
  /** Two concepts are linked when they appear in the same passage; weight counts those passages. */
  edges: { source: number; target: number; weight: number }[];
}

export interface ProcessVisual extends VisualOrigin {
  style: "numbered" | "ordinal" | "arrows";
  /** `text` is the book's own wording. */
  steps: { label: string; text: string }[];
}

export interface ChartVisual extends VisualOrigin {
  kind: "bar" | "line";
  unit: string;
  /** `quote` is the sentence the value was read from. */
  items: { label: string; value: number; quote: string }[];
  total: number | null;
  /** The percentages add up to a whole. */
  whole: boolean;
}

export interface DiagramNode extends VisualOrigin {
  id: string;
  label: string;
  /** Words from the section that support the node, checked against its text. */
  quote: string;
}

export interface DiagramVisual {
  id: number;
  section_id: number;
  kind: "process" | "hierarchy" | "comparison" | "cause_effect";
  title: string;
  model: string;
  created_at: string;
  nodes: DiagramNode[];
  edges: { source: string; target: string; label: string }[];
  /** Items the model proposed that the text did not support. */
  dropped: number;
}

export interface BookVisuals {
  passage_count: number;
  concept_map: ConceptMapData | null;
  processes: ProcessVisual[];
  charts: ChartVisual[];
  diagrams: DiagramVisual[];
}

export interface ConceptView {
  id: number;
  name: string;
  definition: string | null;
  importance: number;
  pages: number[];
  related: string[];
}

const BROWSER_DEV_BACKEND: BackendInfo = { baseUrl: "http://127.0.0.1:8765", token: null };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function resolveBackend(): Promise<BackendInfo> {
  return isTauri() ? invoke<BackendInfo>("backend_info") : BROWSER_DEV_BACKEND;
}

/** Calls a command of the desktop app's Rust engine. */
export type Invoker = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

export class ApiClient {
  constructor(
    private readonly backend: BackendInfo,
    private readonly invoker: Invoker = invoke,
  ) {}

  /** Runs an engine command. The engine reports a failure as a sentence for the learner. */
  private async command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
    if (this.invoker === invoke && !isTauri()) {
      throw new ApiError(0, "This part of the app runs in the desktop app's engine, so it is not available in a browser preview.");
    }
    try {
      return (await this.invoker(name, args)) as T;
    } catch (reason) {
      throw new ApiError(400, typeof reason === "string" ? reason : reason instanceof Error ? reason.message : String(reason));
    }
  }

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    if (this.backend.token) headers.set("x-tutor-token", this.backend.token);
    // FormData bodies must let the browser set the multipart boundary.
    if (init.body && !(init.body instanceof FormData)) headers.set("content-type", "application/json");
    const response = await fetch(`${this.backend.baseUrl}/api${path}`, { ...init, headers });
    if (!response.ok) {
      const body = await response.json().catch(() => null);
      // Validation failures arrive as a list of problems rather than one message.
      const detail =
        typeof body?.detail === "string"
          ? body.detail
          : Array.isArray(body?.detail)
            ? body.detail.map((item: { msg?: string }) => String(item.msg ?? "").replace(/^Value error, /, "")).join(" ")
            : response.statusText;
      throw new ApiError(response.status, detail);
    }
    if (response.status === 204) return undefined as T;
    return response.json() as Promise<T>;
  }

  /**
   * Runs an engine command that reports its progress as events. Aborting the signal stops the
   * work in the engine; the promise then resolves without further events.
   */
  private async events<T>(name: string, args: Record<string, unknown>, onEvent: (event: T) => void, signal?: AbortSignal) {
    const id = crypto.randomUUID();
    let listening = true;
    const deliver = (event: T) => {
      if (listening) onEvent(event);
    };
    // In the desktop app events arrive over a channel; a test's stand-in engine calls back directly.
    let listener: unknown = deliver;
    if (this.invoker === invoke && isTauri()) {
      const channel = new Channel<T>();
      channel.onmessage = deliver;
      listener = channel;
    }
    const stop = () => {
      listening = false;
      void this.invoker("stop_request", { id }).catch(() => undefined);
    };
    if (signal?.aborted) return;
    signal?.addEventListener("abort", stop, { once: true });
    try {
      await this.command<void>(name, { ...args, id, onEvent: listener });
    } finally {
      signal?.removeEventListener("abort", stop);
    }
  }

  health = () => this.request<Health>("/health");
  aiStatus = () => this.command<AiStatus>("ai_status");
  pullModel = (name: string, onEvent: (event: PullEvent) => void, signal?: AbortSignal) =>
    this.events("model_pull", { name }, onEvent, signal);
  deleteModel = (name: string) => this.command<void>("model_delete", { name });
  ask = (bookId: number, question: string, onEvent: (event: AskEvent) => void, signal?: AbortSignal) =>
    this.events("ask", { bookId, question, sectionId: null }, onEvent, signal);
  bookConcepts = (id: number) => this.command<ConceptView[]>("book_concepts", { id });
  /** Asks the local model for more concepts of a chapter. Slow: it runs the model. */
  enrichConcepts = (chapterId: number) => this.command<{ added: number }>("concepts_enrich", { chapterId });
  getSettings = () => this.command<AppSettings>("settings_get");
  putSettings = (settings: AppSettings) => this.command<AppSettings>("settings_put", { settings });

  listBooks = (q: string, collectionId: number | null) =>
    this.command<Book[]>("books_list", { q: q || null, collectionId });
  // Adding and removing a book go to the backend, which stores the PDF and processes it.
  uploadBook = (file: File) => {
    const body = new FormData();
    body.append("file", file);
    return this.request<Book>("/books", { method: "POST", body });
  };
  deleteBook = (id: number) => this.request<void>(`/books/${id}`, { method: "DELETE" });
  setBookCollections = (id: number, collectionIds: number[]) =>
    this.command<Book>("book_set_collections", { id, collectionIds });
  bookStructure = (id: number) => this.command<Chapter[]>("book_structure", { id });
  sectionPassages = (id: number, q = "") => this.command<Passage[]>("section_passages", { id, q: q || null });

  /** Live search: the last word matches as a prefix. */
  search = (q: string, scope: SearchScope) => this.command<SearchResponse>("search_library", { q, scope, live: true });
  recentSearches = () => this.command<RecentSearch[]>("search_recent");
  rememberSearch = (query: string, scope: SearchScope, resultTitle: string | null) =>
    this.command<void>("search_remember", { query, scope: scope.kind, bookId: scope.book_id ?? null, resultTitle });
  clearRecentSearches = () => this.command<void>("search_clear_recent");

  /** What can be drawn from the whole book, or from one chapter or section of it. */
  bookVisuals = (bookId: number, scope: { chapter_id?: number; section_id?: number } = {}) => {
    // A section takes precedence over a chapter.
    return this.command<BookVisuals>("book_visuals", { bookId, chapterId: scope.chapter_id ?? null, sectionId: scope.section_id ?? null });
  };
  /** Asks the local model to plan a diagram of a section. Slow: it runs the model. */
  drawSection = (sectionId: number) => this.command<DiagramVisual>("section_draw", { sectionId });
  deleteDiagram = (id: number) => this.command<void>("diagram_delete", { id });

  climateDatasets = (bookId: number) => this.command<ClimateDataset[]>("climate_list", { bookId });
  climateCandidates = (bookId: number) => this.command<ClimateCandidate[]>("climate_candidates", { bookId });
  /** Creates the data, or replaces it when an id is given. */
  saveClimate = (bookId: number, draft: ClimateDraft, id?: number) =>
    this.command<ClimateDataset>("climate_save", { bookId, draft, id: id ?? null });
  deleteClimate = (id: number) => this.command<void>("climate_delete", { id });
  parseClimate = (text: string) => this.command<ClimateTables>("climate_parse", { text });

  listCollections = () => this.command<Collection[]>("collections_list");
  createCollection = (name: string) => this.command<Collection>("collection_create", { name });
  deleteCollection = (id: number) => this.command<void>("collection_delete", { id });
}
