import { invoke, isTauri } from "@tauri-apps/api/core";

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
}

export interface SearchHit {
  passage_id: number;
  chapter_id: number;
  section_id: number;
  chapter_title: string;
  section_title: string;
  snippet: string;
  start_page: number;
  end_page: number;
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

export class ApiClient {
  constructor(private readonly backend: BackendInfo) {}

  private async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    if (this.backend.token) headers.set("x-tutor-token", this.backend.token);
    // FormData bodies must let the browser set the multipart boundary.
    if (init.body && !(init.body instanceof FormData)) headers.set("content-type", "application/json");
    const response = await fetch(`${this.backend.baseUrl}/api${path}`, { ...init, headers });
    if (!response.ok) {
      const body = await response.json().catch(() => null);
      const detail = typeof body?.detail === "string" ? body.detail : response.statusText;
      throw new ApiError(response.status, detail);
    }
    if (response.status === 204) return undefined as T;
    return response.json() as Promise<T>;
  }

  /** Reads a newline-delimited JSON stream. Aborting the signal stops the work on the backend. */
  private async stream<T>(path: string, body: unknown, onEvent: (event: T) => void, signal?: AbortSignal) {
    const headers = new Headers({ "content-type": "application/json" });
    if (this.backend.token) headers.set("x-tutor-token", this.backend.token);
    const response = await fetch(`${this.backend.baseUrl}/api${path}`, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal,
    });
    if (!response.ok || !response.body) {
      const detail = await response.json().catch(() => null);
      throw new ApiError(response.status, typeof detail?.detail === "string" ? detail.detail : response.statusText);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffered = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffered += decoder.decode(value, { stream: true });
      const lines = buffered.split("\n");
      buffered = lines.pop() ?? "";
      for (const line of lines) if (line) onEvent(JSON.parse(line) as T);
    }
  }

  health = () => this.request<Health>("/health");
  aiStatus = () => this.request<AiStatus>("/ai/status");
  pullModel = (name: string, onEvent: (event: PullEvent) => void, signal?: AbortSignal) =>
    this.stream("/ai/models/pull", { name }, onEvent, signal);
  deleteModel = (name: string) =>
    this.request<void>(`/ai/models?${new URLSearchParams({ name })}`, { method: "DELETE" });
  ask = (bookId: number, question: string, onEvent: (event: AskEvent) => void, signal?: AbortSignal) =>
    this.stream(`/books/${bookId}/ask`, { question }, onEvent, signal);
  bookConcepts = (id: number) => this.request<ConceptView[]>(`/books/${id}/concepts`);
  enrichConcepts = (chapterId: number) =>
    this.request<{ added: number }>(`/chapters/${chapterId}/concepts/enrich`, { method: "POST" });
  getSettings = () => this.request<AppSettings>("/settings");
  putSettings = (settings: AppSettings) =>
    this.request<AppSettings>("/settings", { method: "PUT", body: JSON.stringify(settings) });

  listBooks = (q: string, collectionId: number | null) => {
    const params = new URLSearchParams();
    if (q) params.set("q", q);
    if (collectionId !== null) params.set("collection_id", String(collectionId));
    return this.request<Book[]>(`/books?${params}`);
  };
  uploadBook = (file: File) => {
    const body = new FormData();
    body.append("file", file);
    return this.request<Book>("/books", { method: "POST", body });
  };
  deleteBook = (id: number) => this.request<void>(`/books/${id}`, { method: "DELETE" });
  setBookCollections = (id: number, collectionIds: number[]) =>
    this.request<Book>(`/books/${id}/collections`, {
      method: "PUT",
      body: JSON.stringify({ collection_ids: collectionIds }),
    });
  bookStructure = (id: number) => this.request<Chapter[]>(`/books/${id}/structure`);
  sectionPassages = (id: number) => this.request<Passage[]>(`/sections/${id}/passages`);
  searchBook = (id: number, q: string) =>
    this.request<SearchHit[]>(`/books/${id}/search?${new URLSearchParams({ q })}`);

  listCollections = () => this.request<Collection[]>("/collections");
  createCollection = (name: string) =>
    this.request<Collection>("/collections", { method: "POST", body: JSON.stringify({ name }) });
  deleteCollection = (id: number) => this.request<void>(`/collections/${id}`, { method: "DELETE" });
}
