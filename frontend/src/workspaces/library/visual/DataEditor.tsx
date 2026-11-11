import { type FormEvent, useState } from "react";
import type { ApiClient, ClimateCandidate, ClimateDataset, ClimateDraft, ClimateTables } from "../../../api/client";
import { DIRECTIONS, MONTHS, PRINCIPAL } from "./climate";

interface Props {
  client: ApiClient;
  bookId: number;
  /** The data being changed, or nothing when adding new data. */
  existing?: ClimateDataset;
  onSaved: (dataset: ClimateDataset) => void;
  onCancel?: () => void;
}

type Cells = Record<string, string>;

const text = (value: number) => String(value);
const pageRange = (start: number, end: number) => (start === end ? `p. ${start}` : `pp. ${start}–${end}`);

function toCells(tables: ClimateTables): { max: Cells; min: Cells; share: Cells } {
  return {
    max: Object.fromEntries(tables.months.map((item) => [item.month, text(item.max)])),
    min: Object.fromEntries(tables.months.map((item) => [item.month, text(item.min)])),
    share: Object.fromEntries(tables.winds.map((item) => [item.direction, text(item.share)])),
  };
}

export function DataEditor({ client, bookId, existing, onSaved, onCancel }: Props) {
  const initial = toCells(existing ?? { months: [], winds: [] });
  const [name, setName] = useState(existing?.name ?? "");
  const [max, setMax] = useState<Cells>(initial.max);
  const [min, setMin] = useState<Cells>(initial.min);
  const [share, setShare] = useState<Cells>(initial.share);
  /** The passage the values were read from. Editing a value by hand clears it. */
  const [origin, setOrigin] = useState<{ passage_id: number; label: string } | null>(
    existing?.origin ? { passage_id: existing.origin.passage_id, label: `${existing.origin.section_title}, ${pageRange(existing.origin.start_page, existing.origin.end_page)}` } : null,
  );
  const [pasted, setPasted] = useState("");
  const [candidates, setCandidates] = useState<ClimateCandidate[] | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Eight directions by default; sixteen once the data uses the points in between.
  const directions = Object.keys(share).some((key) => !PRINCIPAL.includes(key) && share[key]) ? DIRECTIONS : PRINCIPAL;
  const report = (reason: unknown) => setMessage(reason instanceof Error ? reason.message : String(reason));

  function fill(tables: ClimateTables) {
    const cells = toCells(tables);
    setMax(cells.max);
    setMin(cells.min);
    setShare(cells.share);
  }

  function edit(set: (update: (cells: Cells) => Cells) => void, key: string | number, value: string) {
    set((cells) => ({ ...cells, [key]: value }));
    setOrigin(null);
  }

  async function readPasted() {
    setMessage(null);
    try {
      const tables = await client.parseClimate(pasted);
      if (tables.months.length === 0 && tables.winds.length === 0) {
        setMessage("No climate data was recognised. Put each month with its maximum and minimum on a line (Jan 32 23), and each wind direction with its percentage (SW 34%).");
        return;
      }
      fill(tables);
      setOrigin(null);
      setMessage(`Read ${tables.months.length} months and ${tables.winds.length} wind directions. Check them against your source before saving.`);
    } catch (reason) {
      report(reason);
    }
  }

  async function findInBook() {
    setMessage(null);
    setBusy(true);
    try {
      setCandidates(await client.climateCandidates(bookId));
    } catch (reason) {
      report(reason);
    } finally {
      setBusy(false);
    }
  }

  function takeCandidate(candidate: ClimateCandidate) {
    fill(candidate);
    setOrigin({ passage_id: candidate.passage_id, label: `${candidate.section_title}, ${pageRange(candidate.start_page, candidate.end_page)}` });
    if (!name.trim()) setName(candidate.section_title);
    setMessage("Values copied from the book. Check them against the page before saving.");
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    setMessage(null);
    const incomplete = MONTHS.find((_, index) => Boolean(max[index + 1]?.trim()) !== Boolean(min[index + 1]?.trim()));
    if (incomplete) {
      setMessage(`${incomplete} needs both a maximum and a minimum.`);
      return;
    }
    const draft: ClimateDraft = {
      name: name.trim(),
      source: origin ? "book" : "entered",
      passage_id: origin?.passage_id ?? null,
      months: MONTHS.flatMap((_, index) => (max[index + 1]?.trim() ? [{ month: index + 1, max: Number(max[index + 1]), min: Number(min[index + 1]) }] : [])),
      winds: DIRECTIONS.flatMap((direction) => (share[direction]?.trim() ? [{ direction, share: Number(share[direction]) }] : [])),
    };
    if ([...draft.months.flatMap((item) => [item.max, item.min]), ...draft.winds.map((item) => item.share)].some(Number.isNaN)) {
      setMessage("Every value must be a number.");
      return;
    }
    setBusy(true);
    try {
      onSaved(await client.saveClimate(bookId, draft, existing?.id));
    } catch (reason) {
      report(reason);
    } finally {
      setBusy(false);
    }
  }

  const cell = "w-full rounded border border-rule bg-surface px-1.5 py-1 text-center text-sm";

  return (
    <form onSubmit={save} aria-label="Climate data" className="max-w-4xl">
      <div className="mb-5 grid gap-6 md:grid-cols-2">
        <section aria-label="Find a table in this book">
          <h3 className="mb-1 text-sm font-medium">From this book</h3>
          <p className="mb-2 text-xs text-muted">Looks for tables of monthly temperatures or wind percentages in the text.</p>
          <button type="button" onClick={() => void findInBook()} disabled={busy} className="rounded-md border border-rule bg-surface px-3 py-1.5 text-sm hover:border-accent">
            Find tables in this book
          </button>
          {candidates && candidates.length === 0 && (
            <p className="mt-2 text-xs text-muted">No table was recognised. Tables printed as images, or laid out unusually, cannot be read; paste or type the values instead.</p>
          )}
          {candidates && candidates.length > 0 && (
            <ul className="mt-2 divide-y divide-rule border-y border-rule text-sm">
              {candidates.map((candidate) => (
                <li key={candidate.passage_id} className="flex items-center justify-between gap-3 py-1.5">
                  <span>
                    {candidate.section_title} <span className="text-xs text-muted">· {pageRange(candidate.start_page, candidate.end_page)} · {candidate.months.length} months, {candidate.winds.length} winds</span>
                  </span>
                  <button type="button" onClick={() => takeCandidate(candidate)} className="shrink-0 text-xs text-accent hover:underline">
                    Use
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section aria-label="Paste a table">
          <h3 className="mb-1 text-sm font-medium">Paste a table</h3>
          <textarea
            value={pasted}
            onChange={(event) => setPasted(event.target.value)}
            aria-label="Pasted climate table"
            placeholder="One row per line: a month with its maximum and minimum, or a wind direction with its percentage."
            rows={4}
            className="w-full rounded-md border border-rule bg-surface px-2 py-1.5 font-mono text-xs"
          />
          <button type="button" onClick={() => void readPasted()} disabled={!pasted.trim()} className="mt-1 rounded-md border border-rule bg-surface px-3 py-1.5 text-sm hover:border-accent disabled:opacity-50">
            Read table
          </button>
        </section>
      </div>

      {message && <p role="status" className="mb-4 rounded-md border border-rule bg-surface p-2.5 text-sm">{message}</p>}

      <label className="mb-4 block text-sm">
        <span className="mb-1 block font-medium">Name of the place or data</span>
        <input value={name} onChange={(event) => setName(event.target.value)} required maxLength={120} className="w-80 max-w-full rounded-md border border-rule bg-surface px-3 py-1.5 text-sm" />
      </label>

      <h3 className="mb-1 text-sm font-medium">Mean monthly temperature, °C</h3>
      <div className="mb-5 overflow-x-auto">
        <table className="text-xs">
          <thead>
            <tr>
              <th />
              {MONTHS.map((month) => (
                <th key={month} className="px-1 pb-1 font-normal text-muted">{month}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {([["Max", max, setMax], ["Min", min, setMin]] as const).map(([label, cells, set]) => (
              <tr key={label}>
                <th className="pr-2 text-left font-normal text-muted">{label}</th>
                {MONTHS.map((month, index) => (
                  <td key={month} className="p-0.5">
                    <input inputMode="decimal" aria-label={`${month} ${label.toLowerCase()}imum`} value={cells[index + 1] ?? ""} onChange={(event) => edit(set, index + 1, event.target.value)} className={`${cell} w-12`} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <h3 className="mb-1 text-sm font-medium">Wind: percentage of the time from each direction</h3>
      <div className="mb-2 flex flex-wrap gap-2">
        {directions.map((direction) => (
          <label key={direction} className="text-center text-xs text-muted">
            {direction}
            <input inputMode="decimal" aria-label={`Wind from ${direction}, percent`} value={share[direction] ?? ""} onChange={(event) => edit(setShare, direction, event.target.value)} className={`${cell} mt-0.5 block w-14`} />
          </label>
        ))}
      </div>
      <p className="mb-5 text-xs text-muted">Leave blank what your source does not give. Whatever is left below 100% counts as calm.</p>

      <p className="mb-3 text-xs text-muted">
        Source: {origin ? `the book, ${origin.label}` : "entered by you"}
      </p>
      <button type="submit" disabled={busy} className="rounded-md bg-accent px-4 py-2 text-sm text-white hover:opacity-90 disabled:opacity-50">
        Save and visualise
      </button>
      {onCancel && (
        <button type="button" onClick={onCancel} className="ml-3 text-sm text-muted hover:text-ink">
          Cancel
        </button>
      )}
    </form>
  );
}
