import { type FormEvent, useCallback, useEffect, useRef, useState } from "react";
import type { AiStatus, ApiClient, AppSettings } from "../api/client";

function gigabytes(bytes: number) {
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

interface Props {
  client: ApiClient;
  settings: AppSettings;
  onChange: (patch: Partial<AppSettings>) => void;
}

const inputClass = "rounded-md border border-rule bg-surface px-2 py-1.5 text-sm";

export function ModelSettings({ client, settings, onChange }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [download, setDownload] = useState<{ status: string; percent: number | null } | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);

  const refresh = useCallback(
    () => client.aiStatus().then(setStatus, (reason) => setError(String(reason))),
    [client],
  );

  useEffect(() => {
    void refresh();
    return () => controller.current?.abort();
  }, [refresh]);

  async function pull(event: FormEvent) {
    event.preventDefault();
    if (!name.trim() || download) return;
    controller.current = new AbortController();
    setError(null);
    setDownload({ status: "Starting", percent: null });
    try {
      await client.pullModel(
        name.trim(),
        (update) => {
          if (update.type === "progress") {
            setDownload({
              status: update.status,
              percent: update.total ? Math.round((update.completed / update.total) * 100) : null,
            });
          } else if (update.type === "error") {
            setError(update.message);
          } else {
            setName("");
          }
        },
        controller.current.signal,
      );
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) setError(String(reason));
    } finally {
      setDownload(null);
      await refresh();
    }
  }

  async function remove(model: string) {
    setConfirming(null);
    try {
      await client.deleteModel(model);
      if (settings.llm_model === model) onChange({ llm_model: null });
      if (settings.embedding_model === model) onChange({ embedding_model: null });
      await refresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  if (!status) return <p className="mt-8 text-sm text-muted">Checking local models…</p>;
  const chatModels = status.models.filter((model) => model.can_chat);
  const embedModels = status.models.filter((model) => model.can_embed);

  return (
    <section aria-label="AI models" className="mt-10 max-w-xl">
      <h2 className="mb-2 font-serif text-xl">AI models</h2>
      <p data-testid="ollama-status" className="text-sm text-muted">
        {status.reachable ? `Ollama ${status.version} is running on this computer.` : status.message}
      </p>
      {error && <p role="alert" className="mt-3 text-sm text-danger">{error}</p>}

      {status.reachable && (
        <>
          <label className="mt-4 flex items-center justify-between gap-8 border-b border-rule py-4 text-sm">
            <span>Language model</span>
            <select
              className={inputClass}
              value={settings.llm_model ?? ""}
              onChange={(event) => onChange({ llm_model: event.target.value || null })}
            >
              <option value="">Automatic ({status.active_model ?? "none"})</option>
              {chatModels.map((model) => (
                <option key={model.name} value={model.name}>
                  {model.name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center justify-between gap-8 border-b border-rule py-4 text-sm">
            <span>Embedding model</span>
            <select
              className={inputClass}
              value={settings.embedding_model ?? ""}
              onChange={(event) => onChange({ embedding_model: event.target.value || null })}
            >
              <option value="">Built-in word matching</option>
              {embedModels.map((model) => (
                <option key={model.name} value={model.name}>
                  {model.name}
                </option>
              ))}
            </select>
          </label>
          <p data-testid="embedding-status" className="mt-2 text-xs text-muted">
            {status.embedding.mode === "model"
              ? `Search understands meaning using ${status.embedding.model}.`
              : "Search matches words and word forms. Install an embedding model for search by meaning."}
          </p>

          {status.running.map((model) => (
            <p key={model.name} className="mt-2 text-xs text-muted">
              {model.name} is loaded on the {model.processor.toUpperCase()}
              {model.vram_bytes ? ` (${gigabytes(model.vram_bytes)} of video memory)` : ""}.
            </p>
          ))}

          <h3 className="mt-6 mb-2 text-sm font-medium">Installed</h3>
          <ul className="divide-y divide-rule border-y border-rule text-sm">
            {status.models.map((model) => (
              <li key={model.name} data-testid="installed-model" className="flex items-center justify-between gap-4 py-2">
                <span>
                  {model.name}{" "}
                  <span className="text-xs text-muted">
                    {gigabytes(model.size_bytes)}
                    {model.can_embed ? " · embeddings" : ""}
                  </span>
                </span>
                {confirming === model.name ? (
                  <span className="flex gap-2 text-xs">
                    <button type="button" className="text-danger" onClick={() => void remove(model.name)}>
                      Confirm remove
                    </button>
                    <button type="button" className="text-muted" onClick={() => setConfirming(null)}>
                      Keep
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    aria-label={`Remove ${model.name}`}
                    className="text-xs text-muted hover:text-danger"
                    onClick={() => setConfirming(model.name)}
                  >
                    Remove
                  </button>
                )}
              </li>
            ))}
          </ul>

          {status.excluded_cloud_models.length > 0 && (
            <p data-testid="excluded-cloud" className="mt-2 text-xs text-muted">
              Hidden because they send text to the internet: {status.excluded_cloud_models.join(", ")}.
            </p>
          )}

          <form onSubmit={pull} className="mt-6 flex items-center gap-2">
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Model name from the Ollama library"
              aria-label="Model to download"
              className={`${inputClass} min-w-0 flex-1`}
            />
            {download ? (
              <button key="cancel" type="button" className={inputClass} onClick={() => controller.current?.abort()}>
                Cancel
              </button>
            ) : (
              <button key="download" type="submit" className="rounded-md bg-accent px-3 py-1.5 text-sm text-white">
                Download
              </button>
            )}
          </form>
          {download && (
            <p role="status" className="mt-2 text-xs text-muted">
              {download.status}
              {download.percent !== null ? ` · ${download.percent}%` : ""}
            </p>
          )}
        </>
      )}
    </section>
  );
}
