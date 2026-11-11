import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { invokerOver } from "../../../api/testing";
import { ApiClient, type ClimateDataset } from "../../../api/client";
import { ClimatePanel } from "./ClimatePanel";
import { alignment, directionName, explainMonth, nearestPrincipal } from "./climate";

const FIGURES = [
  { kind: "wind_rose", title: "Wind rose", needs: "Needs wind directions.", basis: "Drawn exactly from the wind percentages." },
  { kind: "comfort_chart", title: "Temperature and comfort", needs: "Needs monthly temperatures.", basis: "Bars are drawn exactly from the temperatures." },
  { kind: "section", title: "Air through a room", needs: "Needs wind directions.", basis: "A schematic section, not to scale." },
  { kind: "model_3d", title: "3D model", needs: "Needs wind directions.", basis: "A schematic model, not to scale." },
] as const;

function dataset(overrides: Partial<ClimateDataset> = {}): ClimateDataset {
  const winds = overrides.winds ?? [{ direction: "S", share: 22 }, { direction: "SW", share: 34 }, { direction: "W", share: 15 }];
  const months = overrides.months ?? [{ month: 2, max: 33, min: 28 }, { month: 7, max: 26, min: 21 }];
  return {
    id: 1, book_id: 1, name: "Site A", source: "book", created_at: "",
    origin: { passage_id: 9, section_id: 4, section_title: "Site records", start_page: 12, end_page: 12 },
    summary: { prevailing_wind: winds[1] ?? null, calm_share: 29, hottest: { month: 2, max: 33 }, coolest: { month: 7, min: 21 }, mean_daily_range: 5 },
    figures: FIGURES.map((figure) => ({ ...figure, available: figure.kind === "comfort_chart" ? months.length > 0 : winds.length > 0 })),
    ...overrides,
    winds,
    months,
  };
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

function setup(routes: (url: string, init?: RequestInit) => Response | undefined) {
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => routes(url, init) ?? json([]));
  vi.stubGlobal("fetch", fetchMock);
  const onOpenSource = vi.fn();
  render(<ClimatePanel client={new ApiClient({ baseUrl: "http://x", token: null }, invokerOver(fetchMock))} bookId={1} onOpenSource={onOpenSource} />);
  return { fetchMock, onOpenSource };
}

describe("climate helpers", () => {
  it("names directions and judges how an opening faces the wind", () => {
    expect(directionName("SW")).toBe("south-west");
    expect(directionName("NNE")).toBe("north-north-east");
    expect(nearestPrincipal(230)).toBe("SW");
    expect(nearestPrincipal(359)).toBe("N");
    expect(alignment(225, 225)).toBeCloseTo(1);
    expect(alignment(135, 225)).toBeCloseTo(0);
    expect(alignment(45, 225)).toBeCloseTo(-1);
  });

  it("describes a month against the comfort band", () => {
    expect(explainMonth({ month: 2, max: 33, min: 28 }, 20, 27).facts[1]).toMatch(/Even the coolest part of the day is above/);
    expect(explainMonth({ month: 7, max: 26, min: 21 }, 20, 27).facts[1]).toMatch(/whole day sits inside/);
    expect(explainMonth({ month: 1, max: 30, min: 12 }, 20, 27).facts[1]).toMatch(/rise above .* and nights fall below/);
    expect(explainMonth({ month: 1, max: 15, min: 5 }, 20, 27).facts[1]).toMatch(/Even the warmest part of the day is below/);
  });
});

describe("ClimatePanel", () => {
  it("draws the wind rose from the data and explains the chosen direction", async () => {
    const { onOpenSource } = setup((url) => (url.includes("/climate") ? json([dataset()]) : undefined));

    const rose = await screen.findByRole("group", { name: "Wind rose" });
    expect(within(rose).getAllByRole("button")).toHaveLength(3);
    expect(screen.getByText("Choose a petal of the rose to read what it shows.")).toBeInTheDocument();
    await userEvent.click(within(rose).getByRole("button", { name: "Wind from the south-west, 34% of the time" }));

    const explanation = screen.getByRole("region", { name: "Explanation" });
    expect(within(explanation).getByText("It blows from the south-west 34% of the time.")).toBeInTheDocument();
    expect(within(explanation).getByText(/This is the prevailing wind/)).toBeInTheDocument();
    await userEvent.click(within(rose).getByRole("button", { name: /from the west/ }));
    expect(within(explanation).getByText(/3rd most frequent of the 3 directions/)).toBeInTheDocument();

    // The figure says where its numbers came from, and the source can be opened.
    const about = screen.getByRole("region", { name: "About this figure" });
    expect(within(about).getByText("Drawn exactly from the wind percentages.")).toBeInTheDocument();
    await userEvent.click(within(about).getByRole("button", { name: "Site records, p. 12" }));
    expect(onOpenSource).toHaveBeenCalledWith(4, 9);
  });

  it("sets months against a comfort band the learner can change", async () => {
    setup((url) => (url.includes("/climate") ? json([dataset({ source: "entered", origin: null })]) : undefined));
    await userEvent.click(await screen.findByRole("tab", { name: "Temperature and comfort" }));
    await userEvent.click(screen.getByRole("button", { name: "Jul: 21 to 26 degrees Celsius" }));
    const explanation = screen.getByRole("region", { name: "Explanation" });
    expect(within(explanation).getByText(/whole day sits inside the comfort band of 20–27 °C/)).toBeInTheDocument();

    const upper = screen.getByLabelText("Comfort band upper limit");
    await userEvent.clear(upper);
    await userEvent.type(upper, "24");
    expect(within(explanation).getByText(/Days rise above the comfort band of 20–24 °C/)).toBeInTheDocument();
    expect(screen.getByText(/entered by you/)).toBeInTheDocument();
  });

  it("explains the parts of the section and disables figures the data cannot support", async () => {
    setup((url) => (url.includes("/climate") ? json([dataset()]) : undefined));
    await userEvent.click(await screen.findByRole("tab", { name: "Air through a room" }));
    expect(screen.getByText("SW wind · 34% of the time")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Ridge vent" }));
    expect(within(screen.getByRole("region", { name: "Explanation" })).getByText(/Warm air collects under the roof/)).toBeInTheDocument();
  });

  it("offers only the temperature figure when there is no wind data", async () => {
    setup((url) => (url.includes("/climate") ? json([dataset({ winds: [] })]) : undefined));
    expect(await screen.findByRole("tab", { name: "Temperature and comfort" })).toHaveAttribute("aria-selected", "true");
    for (const name of ["Wind rose", "Air through a room", "3D model"]) expect(screen.getByRole("tab", { name })).toBeDisabled();
  });

  it("starts with the editor, reads a pasted table and saves what it shows", async () => {
    const { fetchMock } = setup((url, init) => {
      if (url.endsWith("/climate/parse")) return json({ months: [{ month: 1, max: 32, min: 23 }], winds: [{ direction: "SW", share: 34 }] });
      if (url.endsWith("/books/1/climate") && init?.method === "POST") return json(dataset({ source: "entered", origin: null }), 201);
      if (url.endsWith("/climate/candidates")) return json([]);
      return undefined;
    });

    await userEvent.type(await screen.findByLabelText("Pasted climate table"), "Jan 32 23{enter}SW 34%");
    await userEvent.click(screen.getByRole("button", { name: "Read table" }));
    expect(await screen.findByText(/Read 1 months and 1 wind directions/)).toBeInTheDocument();
    expect(screen.getByLabelText("Jan maximum")).toHaveValue("32");
    expect(screen.getByLabelText("Wind from SW, percent")).toHaveValue("34");

    await userEvent.click(screen.getByRole("button", { name: "Find tables in this book" }));
    expect(await screen.findByText(/No table was recognised/)).toBeInTheDocument();

    await userEvent.type(screen.getByLabelText("Feb maximum"), "33");
    await userEvent.type(screen.getByLabelText("Name of the place or data"), "Site A");
    await userEvent.click(screen.getByRole("button", { name: "Save and visualise" }));
    expect(await screen.findByText("Feb needs both a maximum and a minimum.")).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Feb minimum"), "25");
    await userEvent.click(screen.getByRole("button", { name: "Save and visualise" }));

    await waitFor(() => expect(fetchMock.mock.calls.some(([url, init]) => url.endsWith("/books/1/climate") && init?.method === "POST")).toBe(true));
    const [, init] = fetchMock.mock.calls.find(([url, request]) => url.endsWith("/books/1/climate") && request?.method === "POST")!;
    expect(JSON.parse(init!.body as string)).toEqual({
      name: "Site A", source: "entered", passage_id: null,
      months: [{ month: 1, max: 32, min: 23 }, { month: 2, max: 33, min: 25 }],
      winds: [{ direction: "SW", share: 34 }],
    });
  });
});
