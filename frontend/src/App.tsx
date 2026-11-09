import { useState } from "react";
import type { BackendState } from "./api/useBackend";
import { useBackend } from "./api/useBackend";
import { LibraryWorkspace } from "./workspaces/library/LibraryWorkspace";
import { Upcoming } from "./workspaces/Upcoming";
import { SettingsWorkspace } from "./workspaces/SettingsWorkspace";

const WORKSPACES = [
  { id: "library", label: "My Library", phase: 2, summary: "Upload, organise and process your documents." },
  { id: "learning", label: "Learning Room", phase: 4, summary: "Work through a book with your tutor." },
  { id: "progress", label: "Progress Centre", phase: 6, summary: "See what you understand, with the evidence." },
  { id: "revision", label: "Revision Centre", phase: 6, summary: "Revisit weak concepts on a spaced schedule." },
  { id: "settings", label: "Settings", phase: 1, summary: "Tune how the tutor behaves." },
] as const;

type WorkspaceId = (typeof WORKSPACES)[number]["id"];

function BackendStatus({ state }: { state: BackendState }) {
  const text =
    state.phase === "ready"
      ? `Backend connected · v${state.health.version} · schema ${state.health.schema_version}`
      : state.phase === "connecting"
        ? "Starting backend…"
        : "Backend unavailable";
  const dot =
    state.phase === "ready" ? "bg-accent" : state.phase === "connecting" ? "bg-muted" : "bg-danger";
  return (
    <div role="status" data-testid="backend-status" className="flex items-center gap-2 text-xs text-muted">
      <span className={`size-1.5 rounded-full ${dot}`} />
      {text}
    </div>
  );
}

export default function App() {
  const backend = useBackend();
  const [active, setActive] = useState<WorkspaceId>("library");
  const workspace = WORKSPACES.find((item) => item.id === active)!;

  return (
    <div className="flex h-full">
      <nav aria-label="Workspaces" className="flex w-56 shrink-0 flex-col border-r border-rule px-3 py-5">
        <p className="mb-6 px-3 font-serif text-lg">Interactive Tutor</p>
        <ul className="flex flex-1 flex-col gap-0.5">
          {WORKSPACES.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                aria-current={item.id === active ? "page" : undefined}
                onClick={() => setActive(item.id)}
                className={`w-full rounded-md px-3 py-2 text-left text-sm transition-colors ${
                  item.id === active ? "bg-accent-soft text-accent" : "text-ink hover:bg-rule/50"
                }`}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
        <div className="px-3">
          <BackendStatus state={backend} />
        </div>
      </nav>

      <main className="min-w-0 flex-1 overflow-y-auto px-12 py-10">
        <header className="mb-8">
          <h1 className="font-serif text-3xl">{workspace.label}</h1>
          <p className="mt-1 text-sm text-muted">{workspace.summary}</p>
        </header>

        {backend.phase === "failed" && (
          <p role="alert" className="mb-6 rounded-md border border-danger/30 p-4 text-sm text-danger">
            The local backend could not start. {backend.message}
          </p>
        )}

        {workspace.id === "settings" ? (
          backend.phase === "ready" && <SettingsWorkspace client={backend.client} />
        ) : workspace.id === "library" ? (
          backend.phase === "ready" && <LibraryWorkspace client={backend.client} />
        ) : (
          <Upcoming phase={workspace.phase} />
        )}
      </main>
    </div>
  );
}
