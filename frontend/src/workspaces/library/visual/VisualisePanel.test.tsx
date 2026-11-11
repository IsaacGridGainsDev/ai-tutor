import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { invokerOver } from "../../../api/testing";
import { ApiClient, type BookVisuals, type Chapter } from "../../../api/client";
import { layOutGraph } from "./ContentFigures";
import { VisualisePanel } from "./VisualisePanel";

const ORIGIN = { passage_id: 9, section_id: 4, section_title: "Treating water", start_page: 12, end_page: 12 };
const CHAPTERS: Chapter[] = [
  {
    id: 2, title: "Water", start_page: 10, end_page: 20, detection_method: "outline",
    sections: [
      { id: 4, title: "Treating water", start_page: 12, end_page: 13, passage_count: 2 },
      { id: 5, title: "The water cycle", start_page: 14, end_page: 15, passage_count: 1 },
    ],
  },
];

const VISUALS: BookVisuals = {
  passage_count: 6,
  concept_map: {
    nodes: [
      { ...ORIGIN, id: 1, name: "coagulation", definition: "The clumping of fine particles.", importance: 3, mentions: 2 },
      { ...ORIGIN, id: 2, name: "sedimentation", definition: null, importance: 2, mentions: 1 },
      { ...ORIGIN, id: 3, name: "atmosphere", definition: "The layer of gases around the planet.", importance: 1, mentions: 1 },
    ],
    edges: [{ source: 1, target: 2, weight: 2 }],
  },
  processes: [{ ...ORIGIN, style: "numbered", steps: [
    { label: "Screening removes leaves", text: "Screening removes leaves and large debris." },
    { label: "Coagulation makes fine particles clump", text: "Coagulation makes fine particles clump together." },
    { label: "Filtration passes the water", text: "Filtration passes the water through sand." },
  ] }],
  charts: [{ ...ORIGIN, section_title: "What air is made of", kind: "bar", unit: "%", total: 99.9, whole: true, items: [
    { label: "nitrogen", value: 78, quote: "Dry air is 78% nitrogen, 21% oxygen and 0.9% argon." },
    { label: "oxygen", value: 21, quote: "Dry air is 78% nitrogen, 21% oxygen and 0.9% argon." },
    { label: "argon", value: 0.9, quote: "Dry air is 78% nitrogen, 21% oxygen and 0.9% argon." },
  ] }],
  diagrams: [],
};

const DIAGRAM = {
  id: 7, section_id: 4, kind: "cause_effect", title: "Why particles settle", model: "local:1", created_at: "", dropped: 1,
  nodes: [
    { ...ORIGIN, id: "n1", label: "Coagulant added", quote: "Coagulation makes fine particles clump together" },
    { ...ORIGIN, id: "n2", label: "Clumps settle", quote: "Sedimentation lets the clumps settle" },
  ],
  edges: [{ source: "n1", target: "n2", label: "leads to" }],
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

function setup(routes: (url: string, init?: RequestInit) => Response | undefined, section = CHAPTERS[0].sections[0]) {
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => routes(url, init) ?? json([]));
  vi.stubGlobal("fetch", fetchMock);
  const onOpenSource = vi.fn();
  render(<VisualisePanel client={new ApiClient({ baseUrl: "http://x", token: null }, invokerOver(fetchMock))} bookId={1} chapters={CHAPTERS} section={section} onOpenSource={onOpenSource} />);
  return { fetchMock, onOpenSource };
}

describe("layOutGraph", () => {
  it("is repeatable, keeps nodes in view and draws linked nodes closer than unlinked ones", () => {
    const nodes = ["a", "b", "c", "d", "e"].map((id) => ({ id, label: id }));
    const edges = [{ source: "a", target: "b" }];
    const first = layOutGraph(nodes, edges);
    expect(layOutGraph(nodes, edges)).toEqual(first);
    expect(first.every((point) => point.x >= 0 && point.x <= 680 && point.y >= 0 && point.y <= 480)).toBe(true);
    const distance = (i: number, j: number) => Math.hypot(first[i].x - first[j].x, first[i].y - first[j].y);
    expect(distance(0, 1)).toBeLessThan(distance(0, 3));
  });
});

describe("VisualisePanel", () => {
  it("draws what was found in the open section and explains a concept from the book", async () => {
    const { fetchMock, onOpenSource } = setup((url) => (url.includes("/visuals") ? json(VISUALS) : undefined));

    await screen.findByRole("group", { name: "Concept map" });
    expect(new URL(fetchMock.mock.calls[0][0]).search).toBe("");  // the whole book by default
    await userEvent.selectOptions(screen.getByLabelText("Part of the book"), screen.getByRole("option", { name: /Section open in the reader/ }));
    const map = await screen.findByRole("group", { name: "Concept map" });
    expect(new URL(fetchMock.mock.calls[fetchMock.mock.calls.length - 1][0]).searchParams.get("section_id")).toBe("4");
    expect(within(map).getAllByRole("button").map((node) => node.getAttribute("aria-label"))).toEqual(["coagulation", "sedimentation", "atmosphere"]);

    await userEvent.click(within(map).getByRole("button", { name: "coagulation" }));
    const explanation = screen.getByRole("region", { name: "Explanation" });
    expect(within(explanation).getByText("The clumping of fine particles.")).toBeInTheDocument();
    expect(within(explanation).getByText("Mentioned in 2 passages of this part of the book.")).toBeInTheDocument();
    expect(within(explanation).getByText("Discussed together with: sedimentation.")).toBeInTheDocument();
    await userEvent.click(within(explanation).getByRole("button", { name: /Open in the book · Treating water, p. 12/ }));
    expect(onOpenSource).toHaveBeenCalledWith(4, 9);
    expect(screen.getByText(/not that one causes the other/)).toBeInTheDocument();
  });

  it("shows a process in the book's own words and figures with the sentence they came from", async () => {
    setup((url) => (url.includes("/visuals") ? json(VISUALS) : undefined));
    const list = await screen.findByRole("navigation", { name: "What can be drawn" });
    await userEvent.click(within(list).getByRole("button", { name: "Treating water" }));
    await userEvent.click(within(screen.getByRole("list", { name: "Steps" })).getByRole("button", { name: /Coagulation makes fine particles clump/ }));
    expect(within(screen.getByRole("region", { name: "Explanation" })).getByText("Coagulation makes fine particles clump together.")).toBeInTheDocument();

    await userEvent.click(within(list).getByRole("button", { name: "What air is made of" }));
    expect(screen.getByText("adds up to 99.9%")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "oxygen: 21%" }));
    expect(within(screen.getByRole("region", { name: "Explanation" })).getByText("Dry air is 78% nitrogen, 21% oxygen and 0.9% argon.")).toBeInTheDocument();
  });

  it("changes scope, and says so plainly when nothing can be drawn", async () => {
    const { fetchMock } = setup((url) => {
      if (url.includes("section_id=5")) return json({ passage_count: 1, concept_map: null, processes: [], charts: [], diagrams: [] });
      return url.includes("/visuals") ? json(VISUALS) : undefined;
    });
    await screen.findByRole("group", { name: "Concept map" });
    await userEvent.selectOptions(screen.getByLabelText("Part of the book"), "s:5");
    expect(await screen.findByText(/Nothing here could be drawn automatically/)).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Part of the book"), "book");
    await screen.findByRole("group", { name: "Concept map" });
    const last = new URL(fetchMock.mock.calls[fetchMock.mock.calls.length - 1][0]);
    expect(last.searchParams.has("section_id") || last.searchParams.has("chapter_id")).toBe(false);
    expect(screen.queryByRole("button", { name: "Ask the model to draw this section" })).not.toBeInTheDocument();
  });

  it("asks the model for a diagram, shows what was left out, and reports failure", async () => {
    let fail = true;
    setup((url, init) => {
      if (url.endsWith("/sections/4/diagram") && init?.method === "POST") {
        return fail ? json({ detail: "The model's diagram could not be matched to the text of this section, so nothing was drawn." }, 400) : json(DIAGRAM, 201);
      }
      return url.includes("/visuals") ? json(VISUALS) : undefined;
    });
    await screen.findByRole("group", { name: "Concept map" });
    await userEvent.selectOptions(screen.getByLabelText("Part of the book"), "s:4");
    const ask = await screen.findByRole("button", { name: "Ask the model to draw this section" });
    await userEvent.click(ask);
    expect(await screen.findByRole("alert")).toHaveTextContent("could not be matched to the text");

    fail = false;
    await userEvent.click(ask);
    const diagram = await screen.findByRole("group", { name: "Diagram" });
    expect(screen.getByText("Cause and effect")).toBeInTheDocument();
    expect(screen.getByText(/1 item was left out because the text did not support it/)).toBeInTheDocument();
    await userEvent.click(within(diagram).getByRole("button", { name: "Coagulant added" }));
    const explanation = screen.getByRole("region", { name: "Explanation" });
    expect(within(explanation).getByText("Coagulation makes fine particles clump together")).toBeInTheDocument();
    expect(within(explanation).getByText("→ Clumps settle (leads to)")).toBeInTheDocument();
  });

  it("keeps the climate tool as a separate option", async () => {
    setup((url) => (url.includes("/visuals") ? json(VISUALS) : undefined));
    await userEvent.click(await screen.findByRole("button", { name: "Climate data" }));
    expect(await screen.findByRole("button", { name: "Find tables in this book" })).toBeInTheDocument();
  });
});
