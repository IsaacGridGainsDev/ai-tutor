import { useEffect, useMemo, useState } from "react";
import type { ApiClient, BookVisuals, Chapter, ChartVisual, ConceptMapData, DiagramVisual, ProcessVisual, Section, VisualOrigin } from "../../../api/client";
import { ClimatePanel } from "./ClimatePanel";
import { BarChart, Graph, LineChart, StepFlow } from "./ContentFigures";

interface Props {
  client: ApiClient;
  bookId: number;
  chapters: Chapter[];
  /** The section open in the reader. It is offered first in the list of parts, but the whole book is the default. */
  section: Section | null;
  /** Opens the reader at a passage. */
  onOpenSource: (sectionId: number, passageId: number) => void;
}

type Figure =
  | { key: string; group: "Concepts"; title: string; kind: "map"; data: ConceptMapData }
  | { key: string; group: "Steps"; title: string; kind: "process"; data: ProcessVisual }
  | { key: string; group: "Figures"; title: string; kind: "chart"; data: ChartVisual }
  | { key: string; group: "Drawn by the model"; title: string; kind: "diagram"; data: DiagramVisual };

/** What the learner reads after choosing part of a figure. */
interface Detail {
  title: string;
  lines: string[];
  /** The book's own words. */
  quote?: string;
  origin: VisualOrigin;
}

const pageRange = (start: number, end: number) => (start === end ? `p. ${start}` : `pp. ${start}–${end}`);
const KIND_NAMES = { process: "Process", hierarchy: "Parts of a whole", comparison: "Comparison", cause_effect: "Cause and effect" };

const ABOUT: Record<Figure["kind"], string[]> = {
  map: [
    "Concepts were found in the text automatically when the book was added: from its glossary, from phrases such as “is called”, and from section titles.",
    "A line joins two concepts that appear in the same passage. It shows they are discussed together, not that one causes the other.",
  ],
  process: ["The steps and their order are the book's own: numbered points, “first… then… finally”, or an arrowed sequence in the text.", "Step names are the opening words of each step. Choose a step to read it in full."],
  chart: ["Each value was read from the sentence shown when you choose it. Check it against the page.", "Only figures the text states are drawn. Nothing is estimated."],
  diagram: [
    "Planned by the local AI model from this section. Each item is kept only if the words it quoted were found in the section's text.",
    "The links between items, and the choice of what to include, are the model's reading of the text and can be wrong.",
  ],
};

function figuresOf(visuals: BookVisuals): Figure[] {
  return [
    ...(visuals.concept_map ? [{ key: "map", group: "Concepts", title: "Concept map", kind: "map", data: visuals.concept_map } as const] : []),
    ...visuals.processes.map((data, index) => ({ key: `process-${index}`, group: "Steps", title: data.section_title, kind: "process", data }) as const),
    ...visuals.charts.map((data, index) => ({ key: `chart-${index}`, group: "Figures", title: data.section_title, kind: "chart", data }) as const),
    ...visuals.diagrams.map((data) => ({ key: `diagram-${data.id}`, group: "Drawn by the model", title: data.title, kind: "diagram", data }) as const),
  ];
}

function detailOf(figure: Figure, chosen: string): Detail | null {
  if (figure.kind === "map") {
    const node = figure.data.nodes.find((item) => String(item.id) === chosen);
    if (!node) return null;
    const names = new Map(figure.data.nodes.map((item) => [item.id, item.name]));
    const beside = figure.data.edges
      .filter((edge) => edge.source === node.id || edge.target === node.id)
      .sort((a, b) => b.weight - a.weight)
      .map((edge) => names.get(edge.source === node.id ? edge.target : edge.source));
    return {
      title: node.name,
      lines: [
        node.definition ?? "The book's knowledge map has no definition for this concept yet.",
        `Mentioned in ${node.mentions} ${node.mentions === 1 ? "passage" : "passages"} of this part of the book.`,
        ...(beside.length ? [`Discussed together with: ${beside.slice(0, 6).join(", ")}.`] : []),
      ],
      origin: node,
    };
  }
  if (figure.kind === "process") {
    const step = figure.data.steps[Number(chosen)];
    return step ? { title: `Step ${Number(chosen) + 1}`, lines: [], quote: step.text, origin: figure.data } : null;
  }
  if (figure.kind === "chart") {
    const item = figure.data.items[Number(chosen)];
    return item ? { title: `${item.label}: ${item.value.toLocaleString("en-GB")}${figure.data.unit}`, lines: [], quote: item.quote, origin: figure.data } : null;
  }
  const node = figure.data.nodes.find((item) => item.id === chosen);
  if (!node) return null;
  const labels = new Map(figure.data.nodes.map((item) => [item.id, item.label]));
  const links = figure.data.edges.flatMap((edge) =>
    edge.source === node.id ? [`→ ${labels.get(edge.target)}${edge.label ? ` (${edge.label})` : ""}`]
    : edge.target === node.id ? [`← ${labels.get(edge.source)}${edge.label ? ` (${edge.label})` : ""}`]
    : [],
  );
  return { title: node.label, lines: links, quote: node.quote, origin: node };
}

export function VisualisePanel({ client, bookId, chapters, section, onOpenSource }: Props) {
  const [mode, setMode] = useState<"book" | "climate">("book");
  // The whole book first: it always shows what the upload contains, where a single section may hold nothing drawable.
  const [scope, setScope] = useState("book");
  const [visuals, setVisuals] = useState<BookVisuals | null>(null);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [drawing, setDrawing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const sectionId = scope.startsWith("s:") ? Number(scope.slice(2)) : undefined;
  const chapterId = scope.startsWith("c:") ? Number(scope.slice(2)) : undefined;

  useEffect(() => {
    let stale = false;
    setVisuals(null);
    setError(null);
    client.bookVisuals(bookId, { section_id: sectionId, chapter_id: chapterId }).then(
      (result) => {
        if (!stale) setVisuals(result);
      },
      (reason) => {
        if (!stale) setError(reason instanceof Error ? reason.message : String(reason));
      },
    );
    return () => {
      stale = true;
    };
  }, [client, bookId, sectionId, chapterId]);

  const figures = useMemo(() => (visuals ? figuresOf(visuals) : []), [visuals]);
  const active = figures.find((figure) => figure.key === activeKey) ?? figures[0] ?? null;
  const detail = active && chosen !== null ? detailOf(active, chosen) : null;

  function show(key: string) {
    setActiveKey(key);
    setChosen(null);
  }

  async function draw() {
    if (sectionId === undefined) return;
    setDrawing(true);
    setError(null);
    try {
      const diagram = await client.drawSection(sectionId);
      setVisuals((current) => (current ? { ...current, diagrams: [diagram, ...current.diagrams] } : current));
      show(`diagram-${diagram.id}`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setDrawing(false);
    }
  }

  async function removeDiagram(id: number) {
    try {
      await client.deleteDiagram(id);
      setVisuals((current) => (current ? { ...current, diagrams: current.diagrams.filter((item) => item.id !== id) } : current));
      setActiveKey(null);
      setChosen(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const tab = (value: typeof mode, text: string) => (
    <button
      type="button"
      aria-pressed={mode === value}
      onClick={() => setMode(value)}
      className={`rounded-md border px-3 py-1.5 text-sm ${mode === value ? "border-accent bg-accent-soft text-accent" : "border-rule bg-surface hover:border-accent"}`}
    >
      {text}
    </button>
  );

  return (
    <div>
      <div className="mb-5 flex flex-wrap items-center gap-2">
        {tab("book", "From this book")}
        {tab("climate", "Climate data")}
        {mode === "book" && (
          <select aria-label="Part of the book" value={scope} onChange={(event) => { setScope(event.target.value); show(""); }} className="ml-2 max-w-sm rounded-md border border-rule bg-surface px-2 py-1.5 text-sm">
            <option value="book">Whole book</option>
            {section && <option value={`s:${section.id}`}>Section open in the reader: {section.title}</option>}
            {chapters.map((chapter) => (
              <optgroup key={chapter.id} label={chapter.title}>
                <option value={`c:${chapter.id}`}>All of {chapter.title}</option>
                {chapter.sections.filter((item) => item.passage_count > 0).map((item) => (
                  <option key={item.id} value={`s:${item.id}`}>{item.title}</option>
                ))}
              </optgroup>
            ))}
          </select>
        )}
      </div>

      {mode === "climate" ? (
        <ClimatePanel client={client} bookId={bookId} onOpenSource={onOpenSource} />
      ) : (
        <>
          {error && <p role="alert" className="mb-4 rounded-md border border-danger/30 p-3 text-sm text-danger">{error}</p>}
          {visuals === null && !error && <p className="text-sm text-muted">Reading this part of the book…</p>}

          {visuals && (
            <div className="grid gap-8 lg:grid-cols-[13rem_minmax(0,1fr)_19rem]">
              <nav aria-label="What can be drawn" className="text-sm">
                {figures.length === 0 && (
                  <p className="mb-4 text-muted">
                    Nothing here could be drawn automatically: this part of the book has no listed concepts, no step-by-step process and no series of figures in its text.
                    {sectionId === undefined ? " Try a single section, or the whole book." : " Try the whole book for its concept map."}
                  </p>
                )}
                {(["Concepts", "Steps", "Figures", "Drawn by the model"] as const).map((group) => {
                  const members = figures.filter((figure) => figure.group === group);
                  return members.length === 0 ? null : (
                    <div key={group} className="mb-4">
                      <h3 className="mb-1 text-xs uppercase tracking-wide text-muted">{group}</h3>
                      <ul className="space-y-0.5">
                        {members.map((figure) => (
                          <li key={figure.key}>
                            <button
                              type="button"
                              aria-current={active?.key === figure.key ? "true" : undefined}
                              onClick={() => show(figure.key)}
                              className={`w-full rounded-md px-2 py-1 text-left ${active?.key === figure.key ? "bg-accent-soft text-accent" : "hover:bg-rule/50"}`}
                            >
                              {figure.title}
                            </button>
                          </li>
                        ))}
                      </ul>
                    </div>
                  );
                })}
                {sectionId !== undefined && (
                  <div className="border-t border-rule pt-3">
                    <button type="button" onClick={() => void draw()} disabled={drawing} className="w-full rounded-md border border-rule bg-surface px-2.5 py-1.5 text-left hover:border-accent disabled:opacity-60">
                      {drawing ? "The model is reading the section…" : "Ask the model to draw this section"}
                    </button>
                    <p className="mt-1 text-xs text-muted">Uses the local AI model and can take a minute or more.</p>
                  </div>
                )}
              </nav>

              {active && (
                <>
                  <figure aria-label={active.title} className="min-w-0">
                    <figcaption className="mb-3">
                      <span className="font-serif text-lg">{active.title}</span>
                      {active.kind === "diagram" && <span className="ml-2 text-xs text-muted">{KIND_NAMES[active.data.kind]}</span>}
                      {active.kind === "chart" && active.data.whole && <span className="ml-2 text-xs text-muted">adds up to {active.data.total}%</span>}
                    </figcaption>
                    {active.kind === "map" && (
                      <Graph
                        label="Concept map"
                        nodes={active.data.nodes.map((node) => ({ id: String(node.id), label: node.name, weight: Math.min(node.mentions / 4, 1) }))}
                        edges={active.data.edges.map((edge) => ({ source: String(edge.source), target: String(edge.target), weight: edge.weight }))}
                        selected={chosen}
                        onSelect={setChosen}
                      />
                    )}
                    {active.kind === "process" && (
                      <StepFlow label="Steps" steps={active.data.steps.map((step, index) => ({ id: String(index), label: step.label }))} selected={chosen} onSelect={setChosen} />
                    )}
                    {active.kind === "chart" && active.data.kind === "bar" && (
                      <BarChart label="Figures from the text" items={active.data.items} unit={active.data.unit} selected={chosen === null ? null : Number(chosen)} onSelect={(index) => setChosen(String(index))} />
                    )}
                    {active.kind === "chart" && active.data.kind === "line" && (
                      <LineChart label="Figures from the text" items={active.data.items} unit={active.data.unit} selected={chosen === null ? null : Number(chosen)} onSelect={(index) => setChosen(String(index))} />
                    )}
                    {active.kind === "diagram" && active.data.kind === "process" && (
                      <StepFlow
                        label="Steps"
                        steps={active.data.nodes.map((node) => ({ id: node.id, label: node.label, link: active.data.edges.find((edge) => edge.target === node.id)?.label }))}
                        selected={chosen}
                        onSelect={setChosen}
                      />
                    )}
                    {active.kind === "diagram" && active.data.kind !== "process" && (
                      <Graph label="Diagram" directed nodes={active.data.nodes.map((node) => ({ id: node.id, label: node.label }))} edges={active.data.edges} selected={chosen} onSelect={setChosen} />
                    )}
                    {active.kind === "diagram" && (
                      <button type="button" onClick={() => void removeDiagram(active.data.id)} className="mt-3 text-xs text-muted hover:text-danger">
                        Remove this diagram
                      </button>
                    )}
                  </figure>

                  <aside className="text-sm">
                    <section aria-label="Explanation" aria-live="polite" className="mb-5 min-h-28 rounded-md border border-rule bg-surface p-3">
                      {detail ? (
                        <>
                          <h3 className="mb-1 font-medium">{detail.title}</h3>
                          {detail.quote && <blockquote className="mb-2 border-l-2 border-accent pl-2.5 font-serif leading-relaxed">{detail.quote}</blockquote>}
                          {detail.lines.map((line) => (
                            <p key={line} className="mb-1.5">{line}</p>
                          ))}
                          <button type="button" onClick={() => onOpenSource(detail.origin.section_id, detail.origin.passage_id)} className="mt-1 text-xs text-accent hover:underline">
                            Open in the book · {detail.origin.section_title}, {pageRange(detail.origin.start_page, detail.origin.end_page)}
                          </button>
                        </>
                      ) : (
                        <p className="text-muted">Choose part of the drawing to read what the book says about it.</p>
                      )}
                    </section>
                    <section aria-label="About this figure" className="text-xs leading-relaxed text-muted">
                      <h3 className="mb-1 uppercase tracking-wide">About this figure</h3>
                      {ABOUT[active.kind].map((line) => (
                        <p key={line} className="mb-1.5">{line}</p>
                      ))}
                      {active.kind === "diagram" && (
                        <p>
                          Model: {active.data.model}.
                          {active.data.dropped > 0 && ` ${active.data.dropped} ${active.data.dropped === 1 ? "item was" : "items were"} left out because the text did not support ${active.data.dropped === 1 ? "it" : "them"}.`}
                        </p>
                      )}
                    </section>
                  </aside>
                </>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
