import { type ReactNode, useEffect, useState } from "react";
import type { ApiClient, AppSettings } from "../api/client";
import { ModelSettings } from "./ModelSettings";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex items-center justify-between gap-8 border-b border-rule py-4 text-sm">
      <span>{label}</span>
      {children}
    </label>
  );
}

const selectClass = "rounded-md border border-rule bg-surface px-2 py-1.5 text-sm";

export function SettingsWorkspace({ client }: { client: ApiClient }) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    client.getSettings().then(setSettings, (reason) => setError(String(reason)));
  }, [client]);

  async function update(patch: Partial<AppSettings>) {
    if (!settings) return;
    const previous = settings;
    setSettings({ ...settings, ...patch });
    try {
      setSettings(await client.putSettings({ ...settings, ...patch }));
      setError(null);
    } catch (reason) {
      setSettings(previous);
      setError(String(reason));
    }
  }

  if (error && !settings) return <p role="alert" className="text-sm text-danger">{error}</p>;
  if (!settings) return <p className="text-sm text-muted">Loading settings…</p>;

  return (
    <>
    <form className="max-w-xl" onSubmit={(event) => event.preventDefault()}>
      {error && <p role="alert" className="mb-4 text-sm text-danger">{error}</p>}
      <Field label={`Reading speed (${settings.reading_speed.toFixed(2)}×)`}>
        <input
          type="range"
          min={0.5}
          max={2}
          step={0.05}
          value={settings.reading_speed}
          onChange={(event) => void update({ reading_speed: Number(event.target.value) })}
        />
      </Field>
      <Field label="Explanation depth">
        <select
          className={selectClass}
          value={settings.explanation_depth}
          onChange={(event) =>
            void update({ explanation_depth: event.target.value as AppSettings["explanation_depth"] })
          }
        >
          <option value="brief">Brief</option>
          <option value="standard">Standard</option>
          <option value="deep">Deep</option>
        </select>
      </Field>
      <Field label="Assessment difficulty">
        <select
          className={selectClass}
          value={settings.assessment_difficulty}
          onChange={(event) =>
            void update({
              assessment_difficulty: event.target.value as AppSettings["assessment_difficulty"],
            })
          }
        >
          <option value="gentle">Gentle</option>
          <option value="standard">Standard</option>
          <option value="rigorous">Rigorous</option>
        </select>
      </Field>
      <Field label="Compute preference">
        <select
          className={selectClass}
          value={settings.compute_preference}
          onChange={(event) =>
            void update({ compute_preference: event.target.value as AppSettings["compute_preference"] })
          }
        >
          <option value="auto">Automatic</option>
          <option value="gpu">Prefer GPU</option>
          <option value="cpu">CPU only</option>
        </select>
      </Field>
      <Field label="Document storage folder">
        <input
          key={settings.document_storage_path ?? "default"}
          type="text"
          defaultValue={settings.document_storage_path ?? ""}
          placeholder="Default (app data folder)"
          className={`${selectClass} w-72`}
          onBlur={(event) => {
            const value = event.target.value.trim() || null;
            if (value !== settings.document_storage_path) void update({ document_storage_path: value });
          }}
        />
      </Field>
    </form>
    <ModelSettings client={client} settings={settings} onChange={(patch) => void update(patch)} />
    </>
  );
}
