import { type FormEvent, useEffect, useRef, useState } from "react";
import type { AiStatus, AnswerSegment, ApiClient, AskEvent, Citation } from "../../api/client";

type Answer = Extract<AskEvent, { type: "answer" }>;

const KIND_LABEL: Record<AnswerSegment["kind"], string> = {
  supported: "From the book",
  background: "General knowledge",
  interpretation: "Tutor's interpretation",
  not_found: "Not in this document",
};

const KIND_STYLE: Record<AnswerSegment["kind"], string> = {
  supported: "border-accent",
  background: "border-rule",
  interpretation: "border-rule border-dashed",
  not_found: "border-danger",
};

function pages(citation: Citation) {
  return citation.start_page === citation.end_page
    ? `p. ${citation.start_page}`
    : `pp. ${citation.start_page}–${citation.end_page}`;
}

interface Props {
  client: ApiClient;
  bookId: number;
  onOpenCitation: (citation: Citation) => void;
}

export function AskPanel({ client, bookId, onOpenCitation }: Props) {
  const [question, setQuestion] = useState("");
  const [stage, setStage] = useState<"idle" | "retrieving" | "generating">("idle");
  const [tokens, setTokens] = useState(0);
  const [seconds, setSeconds] = useState(0);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<AiStatus | null>(null);
  const controller = useRef<AbortController | null>(null);

  useEffect(() => {
    client.aiStatus().then(setStatus, (reason) => setError(String(reason)));
    return () => controller.current?.abort();
  }, [client]);

  const busy = stage !== "idle";
  useEffect(() => {
    if (!busy) return;
    setSeconds(0);
    const timer = setInterval(() => setSeconds((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, [busy]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!question.trim() || busy) return;
    controller.current = new AbortController();
    setAnswer(null);
    setError(null);
    setTokens(0);
    setStage("retrieving");
    try {
      await client.ask(
        bookId,
        question,
        (update) => {
          if (update.type === "stage") setStage(update.stage);
          else if (update.type === "progress") setTokens(update.tokens);
          else if (update.type === "answer") setAnswer(update);
          else setError(update.message);
        },
        controller.current.signal,
      );
      client.aiStatus().then(setStatus, () => undefined);
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(reason instanceof Error ? reason.message : String(reason));
      }
    } finally {
      setStage("idle");
    }
  }

  const running = status?.running.find((model) => model.name === status.active_model);

  return (
    <section aria-label="Ask about this book" className="max-w-2xl">
      <p data-testid="model-status" className="mb-4 text-xs text-muted">
        {status === null
          ? "Checking local model…"
          : !status.reachable || !status.active_model
            ? (status.message ?? "No local model available.")
            : `Local model: ${status.active_model}${running ? ` · running on ${running.processor.toUpperCase()}` : ""}`}
      </p>

      <form onSubmit={submit} className="flex gap-2">
        <input
          value={question}
          onChange={(event) => setQuestion(event.target.value)}
          placeholder="Ask a question about this document"
          aria-label="Your question"
          className="min-w-0 flex-1 rounded-md border border-rule bg-surface px-3 py-2 text-sm"
        />
        {/* Distinct keys: a reused element would become type=submit mid-click and resubmit the form. */}
        {busy ? (
          <button
            key="stop"
            type="button"
            onClick={() => controller.current?.abort()}
            className="rounded-md border border-rule px-4 py-2 text-sm hover:text-danger"
          >
            Stop
          </button>
        ) : (
          <button key="ask" type="submit" className="rounded-md bg-accent px-4 py-2 text-sm text-white hover:opacity-90">
            Ask
          </button>
        )}
      </form>

      {busy && (
        <p role="status" data-testid="ask-stage" className="mt-3 text-xs text-muted">
          {stage === "retrieving" ? "Searching the document…" : "The tutor is writing an answer…"} {seconds}s
          {tokens > 0 ? ` · ${tokens} tokens` : ""}
        </p>
      )}

      {error && (
        <p role="alert" className="mt-4 rounded-md border border-danger/30 p-3 text-sm text-danger">
          {error}
        </p>
      )}

      {answer && (
        <div data-testid="answer" className="mt-6 space-y-4">
          {answer.segments.map((segment, index) => (
            <div key={index} className={`border-l-2 pl-4 ${KIND_STYLE[segment.kind]}`}>
              <p data-testid="segment-kind" className="mb-1 text-xs uppercase tracking-wide text-muted">
                {KIND_LABEL[segment.kind]}
              </p>
              <p className="font-serif leading-relaxed">{segment.text}</p>
              {segment.quote && <blockquote className="mt-2 text-sm italic text-muted">“{segment.quote}”</blockquote>}
              {segment.citation && (
                <button
                  type="button"
                  onClick={() => onOpenCitation(segment.citation!)}
                  className="mt-1 text-xs text-accent hover:underline"
                >
                  {segment.citation.section_title} · {pages(segment.citation)}
                </button>
              )}
              {segment.downgraded && (
                <p className="mt-1 text-xs text-muted">
                  The tutor claimed this came from the book, but its quote could not be verified.
                </p>
              )}
            </div>
          ))}
          {answer.stats && (
            <p className="text-xs text-muted">
              {answer.model} · {Math.round(answer.stats.total_seconds)}s
            </p>
          )}
        </div>
      )}
    </section>
  );
}
