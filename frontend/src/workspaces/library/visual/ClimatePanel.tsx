import { Suspense, lazy, useCallback, useEffect, useState } from "react";
import type { ApiClient, ClimateDataset, FigureKind } from "../../../api/client";
import { DataEditor } from "./DataEditor";
import { ComfortChart, RoomSection, WindRose } from "./Figures";
import { type Explanation, MONTHS, type PartId, explainMonth, explainPart, explainWind } from "./climate";

// Three.js is large, so it loads only when the 3D model is opened.
const Model3D = lazy(() => import("./Model3D"));

interface Props {
  client: ApiClient;
  bookId: number;
  /** Opens the reader at the passage the data was read from. */
  onOpenSource: (sectionId: number, passageId: number) => void;
}

const pageRange = (start: number, end: number) => (start === end ? `p. ${start}` : `pp. ${start}–${end}`);

export function ClimatePanel({ client, bookId, onOpenSource }: Props) {
  const [datasets, setDatasets] = useState<ClimateDataset[] | null>(null);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [editing, setEditing] = useState<"new" | "existing" | null>(null);
  const [figure, setFigure] = useState<FigureKind | null>(null);
  const [wind, setWind] = useState<string | null>(null);
  const [month, setMonth] = useState<number | null>(null);
  const [part, setPart] = useState<PartId | null>(null);
  const [band, setBand] = useState({ low: 20, high: 27 });
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(
    () => client.climateDatasets(bookId).then(setDatasets, (reason) => setError(reason instanceof Error ? reason.message : String(reason))),
    [client, bookId],
  );
  useEffect(() => {
    void load();
  }, [load]);

  const active = datasets?.find((item) => item.id === activeId) ?? datasets?.[0] ?? null;
  const offers = active?.figures ?? [];
  const shown = offers.find((offer) => offer.kind === figure && offer.available) ?? offers.find((offer) => offer.available) ?? null;

  function choose(id: number) {
    setActiveId(id);
    setWind(null);
    setMonth(null);
    setPart(null);
    setConfirming(false);
  }

  async function remove() {
    if (!active) return;
    try {
      await client.deleteClimate(active.id);
      setConfirming(false);
      setActiveId(null);
      await load();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  if (error) return <p role="alert" className="text-sm text-danger">{error}</p>;
  if (datasets === null) return <p className="text-sm text-muted">Loading…</p>;

  if (editing || !active) {
    return (
      <div>
        {!active && (
          <p className="mb-5 max-w-2xl text-sm text-muted">
            Turn the climate figures in your material into drawings you can explore. Start with the monthly temperatures and wind
            directions for a place: find them in this book, paste them, or type them in.
          </p>
        )}
        <DataEditor
          client={client}
          bookId={bookId}
          existing={editing === "existing" && active ? active : undefined}
          onCancel={active ? () => setEditing(null) : undefined}
          onSaved={(saved) => {
            setEditing(null);
            choose(saved.id);
            void load();
          }}
        />
      </div>
    );
  }

  const selectedWind = active.winds.find((item) => item.direction === wind);
  const selectedMonth = active.months.find((item) => item.month === month);
  const prevailing = active.summary.prevailing_wind;
  const explanation: Explanation | null =
    shown?.kind === "wind_rose"
      ? selectedWind ? explainWind(selectedWind, active.winds) : null
      : shown?.kind === "comfort_chart"
        ? selectedMonth ? explainMonth(selectedMonth, band.low, band.high) : null
        : part ? explainPart(part) : null;
  const prompt =
    shown?.kind === "wind_rose" ? "Choose a petal of the rose to read what it shows."
    : shown?.kind === "comfort_chart" ? "Choose a month to read how it sits against the comfort band."
    : "Choose a part of the building to read what it does.";
  const { summary } = active;

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center gap-2 text-sm">
        <select aria-label="Climate data" value={active.id} onChange={(event) => choose(Number(event.target.value))} className="rounded-md border border-rule bg-surface px-2 py-1.5">
          {datasets.map((item) => (
            <option key={item.id} value={item.id}>{item.name}</option>
          ))}
        </select>
        <button type="button" onClick={() => setEditing("existing")} className="rounded-md border border-rule bg-surface px-2.5 py-1.5 hover:border-accent">Edit data</button>
        <button type="button" onClick={() => setEditing("new")} className="rounded-md border border-rule bg-surface px-2.5 py-1.5 hover:border-accent">Add data</button>
        {confirming ? (
          <button type="button" onClick={() => void remove()} className="rounded-md border border-danger px-2.5 py-1.5 text-danger">Confirm remove</button>
        ) : (
          <button type="button" onClick={() => setConfirming(true)} className="px-1 text-muted hover:text-danger">Remove</button>
        )}
      </div>

      <dl aria-label="What the data says" className="mb-5 flex flex-wrap gap-x-8 gap-y-2 text-sm">
        {prevailing && (
          <div><dt className="text-xs text-muted">Prevailing wind</dt><dd>{prevailing.direction} · {prevailing.share}%</dd></div>
        )}
        {summary.calm_share !== null && summary.calm_share > 0 && (
          <div><dt className="text-xs text-muted">Calm or unlisted</dt><dd>{summary.calm_share}%</dd></div>
        )}
        {summary.hottest && (
          <div><dt className="text-xs text-muted">Highest maximum</dt><dd>{summary.hottest.max} °C · {MONTHS[summary.hottest.month - 1]}</dd></div>
        )}
        {summary.coolest && (
          <div><dt className="text-xs text-muted">Lowest minimum</dt><dd>{summary.coolest.min} °C · {MONTHS[summary.coolest.month - 1]}</dd></div>
        )}
        {summary.mean_daily_range !== null && (
          <div><dt className="text-xs text-muted">Mean daily range</dt><dd>{summary.mean_daily_range} °C</dd></div>
        )}
      </dl>

      <div role="tablist" aria-label="Figures" className="mb-4 flex flex-wrap gap-1.5">
        {offers.map((offer) => (
          <button
            key={offer.kind}
            type="button"
            role="tab"
            aria-selected={shown?.kind === offer.kind}
            disabled={!offer.available}
            title={offer.available ? undefined : offer.needs}
            onClick={() => setFigure(offer.kind)}
            className={`rounded-md border px-3 py-1.5 text-sm disabled:opacity-40 ${
              shown?.kind === offer.kind ? "border-accent bg-accent-soft text-accent" : "border-rule bg-surface hover:border-accent"
            }`}
          >
            {offer.title}
          </button>
        ))}
      </div>

      {shown && (
        <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_19rem]">
          <figure aria-label={shown.title} className="min-w-0">
            {shown.kind === "wind_rose" && <WindRose winds={active.winds} selected={wind} onSelect={setWind} />}
            {shown.kind === "comfort_chart" && (
              <>
                <ComfortChart months={active.months} low={band.low} high={band.high} selected={month} onSelect={setMonth} />
                <div className="mt-2 flex items-center gap-2 text-xs text-muted">
                  Comfort band from
                  <input type="number" aria-label="Comfort band lower limit" value={band.low} onChange={(event) => setBand({ ...band, low: Number(event.target.value) })} className="w-16 rounded border border-rule bg-surface px-1.5 py-1 text-ink" />
                  to
                  <input type="number" aria-label="Comfort band upper limit" value={band.high} onChange={(event) => setBand({ ...band, high: Number(event.target.value) })} className="w-16 rounded border border-rule bg-surface px-1.5 py-1 text-ink" />
                  °C
                </div>
              </>
            )}
            {shown.kind === "section" && prevailing && <RoomSection wind={prevailing} selected={part} onSelect={setPart} />}
            {shown.kind === "model_3d" && (
              <Suspense fallback={<p className="text-sm text-muted">Loading the 3D model…</p>}>
                <Model3D key={active.id} winds={active.winds} selected={part} onSelect={setPart} />
              </Suspense>
            )}
          </figure>

          <aside className="text-sm">
            <section aria-label="Explanation" aria-live="polite" className="mb-5 min-h-28 rounded-md border border-rule bg-surface p-3">
              {explanation ? (
                <>
                  <h3 className="mb-1 font-medium">{explanation.title}</h3>
                  {explanation.facts.map((fact) => (
                    <p key={fact} className="mb-1.5">{fact}</p>
                  ))}
                  <p className="mt-2 border-t border-rule pt-2 text-muted">
                    <span className="text-xs uppercase tracking-wide">General principle</span>
                    <br />
                    {explanation.principle}
                  </p>
                </>
              ) : (
                <p className="text-muted">{prompt}</p>
              )}
            </section>

            <section aria-label="About this figure" className="text-xs leading-relaxed text-muted">
              <h3 className="mb-1 uppercase tracking-wide">About this figure</h3>
              <p className="mb-1.5">
                <span className="text-ink">Data: </span>
                {active.origin ? (
                  <>
                    read from the book,{" "}
                    <button type="button" onClick={() => onOpenSource(active.origin!.section_id, active.origin!.passage_id)} className="text-accent hover:underline">
                      {active.origin.section_title}, {pageRange(active.origin.start_page, active.origin.end_page)}
                    </button>
                    . Check it against the page.
                  </>
                ) : (
                  "entered by you. It has not been checked against the book."
                )}
              </p>
              <p className="mb-1.5"><span className="text-ink">Drawing: </span>{shown.basis}</p>
              <p><span className="text-ink">Explanations: </span>statements about the numbers are computed from the data. Design principles are general knowledge, not quotations from your book.</p>
            </section>
          </aside>
        </div>
      )}
    </div>
  );
}
