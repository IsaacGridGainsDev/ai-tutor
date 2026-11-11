import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { type AiStatus, ApiClient, type AppSettings, type AskEvent } from "../../api/client";
import { invokerOver } from "../../api/testing";
import { ModelSettings } from "../ModelSettings";
import { AskPanel } from "./AskPanel";

const STATUS: AiStatus = {
  reachable: true,
  version: "1.0",
  message: null,
  active_model: "local:1",
  models: [
    { name: "local:1", size_bytes: 2 * 1024 ** 3, family: "f", parameter_size: "1B", quantization: "Q4", can_chat: true, can_embed: false },
  ],
  excluded_cloud_models: ["remote:cloud"],
  running: [{ name: "local:1", processor: "cpu", vram_bytes: 0, size_bytes: 10 }],
  embedding: { mode: "hashing", model: null },
};

function client() {
  return new ApiClient({ baseUrl: "http://x", token: "t" }, invokerOver((url, init) => fetch(url, init)));
}

function stream(events: unknown[]) {
  return new Response(events.map((event) => JSON.stringify(event)).join("\n") + "\n");
}

function json(body: unknown) {
  return new Response(JSON.stringify(body));
}

const ANSWER: AskEvent = {
  type: "answer",
  found_in_document: true,
  concepts: [],
  model: "local:1",
  stats: { total_seconds: 12, completion_tokens: 40 },
  segments: [
    {
      kind: "supported",
      text: "A base case stops recursion.",
      quote: "a conditional branch",
      downgraded: false,
      citation: { passage_id: 5, section_id: 2, chapter_title: "C", section_title: "Recursion", start_page: 65, end_page: 66 },
    },
    { kind: "background", text: "Most languages allow this.", quote: null, citation: null, downgraded: false },
    { kind: "interpretation", text: "A claim.", quote: null, citation: null, downgraded: true },
  ],
};

describe("AskPanel", () => {
  it("labels each kind of claim, shows citations and warns about unverified quotes", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) => (url.includes("/ai/status") ? json(STATUS) : stream([ANSWER]))));
    const onOpen = vi.fn();
    render(<AskPanel client={client()} bookId={1} onOpenCitation={onOpen} />);
    expect(await screen.findByTestId("model-status")).toHaveTextContent("running on CPU");

    await userEvent.type(screen.getByLabelText("Your question"), "What stops recursion?{enter}");
    await screen.findByTestId("answer");

    expect(screen.getAllByTestId("segment-kind").map((node) => node.textContent)).toEqual([
      "From the book", "General knowledge", "Tutor's interpretation",
    ]);
    expect(screen.getByText(/could not be verified/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Recursion · pp. 65–66" }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ passage_id: 5 }));
  });

  it("shows model errors from the stream", async () => {
    vi.stubGlobal("fetch", vi.fn(async (url: string) =>
      url.includes("/ai/status") ? json(STATUS) : stream([{ type: "error", message: "Ollama is not running." }]),
    ));
    render(<AskPanel client={client()} bookId={1} onOpenCitation={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Your question"), "Anything here?{enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent("Ollama is not running.");
  });

  it("aborts the request when Stop is pressed and shows no error", async () => {
    let aborted = false;
    vi.stubGlobal("fetch", vi.fn((url: string, init?: RequestInit) => {
      if (url.includes("/ai/status")) return Promise.resolve(json(STATUS));
      return new Promise<Response>((_, reject) => {
        init!.signal!.addEventListener("abort", () => {
          aborted = true;
          reject(new DOMException("Aborted", "AbortError"));
        });
      });
    }));
    render(<AskPanel client={client()} bookId={1} onOpenCitation={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Your question"), "Long question?{enter}");
    await userEvent.click(await screen.findByRole("button", { name: "Stop" }));
    await waitFor(() => expect(aborted).toBe(true));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Ask" })).toBeInTheDocument();
  });

  it("does not reuse the Stop button's element as the submit button", async () => {
    vi.stubGlobal("fetch", vi.fn((url: string, init?: RequestInit) => {
      if (url.includes("/ai/status")) return Promise.resolve(json(STATUS));
      return new Promise<Response>((_, reject) => {
        init!.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")));
      });
    }));
    render(<AskPanel client={client()} bookId={1} onOpenCitation={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Your question"), "Long question?{enter}");
    const stop = await screen.findByRole("button", { name: "Stop" });
    await userEvent.click(stop);
    const ask = await screen.findByRole("button", { name: "Ask" });
    // A reused element flips to type=submit during the click, and the browser then submits the form.
    expect(ask).not.toBe(stop);
  });

  it("handles stream chunks that split a line in two", async () => {
    const encoder = new TextEncoder();
    const whole = JSON.stringify(ANSWER) + "\n";
    const body = new ReadableStream({
      start(control) {
        control.enqueue(encoder.encode(whole.slice(0, 30)));
        control.enqueue(encoder.encode(whole.slice(30)));
        control.close();
      },
    });
    vi.stubGlobal("fetch", vi.fn(async (url: string) => (url.includes("/ai/status") ? json(STATUS) : new Response(body))));
    render(<AskPanel client={client()} bookId={1} onOpenCitation={vi.fn()} />);
    await userEvent.type(screen.getByLabelText("Your question"), "Split me?{enter}");
    expect(await screen.findByTestId("answer")).toBeInTheDocument();
  });
});

describe("ModelSettings", () => {
  const settings = { llm_model: null, embedding_model: null } as AppSettings;

  it("lists local models, names hidden cloud models and states the embedding mode", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => json(STATUS)));
    render(<ModelSettings client={client()} settings={settings} onChange={vi.fn()} />);
    expect(await screen.findByTestId("installed-model")).toHaveTextContent("local:1");
    expect(screen.getByTestId("excluded-cloud")).toHaveTextContent("remote:cloud");
    expect(screen.getByTestId("embedding-status")).toHaveTextContent("matches words");
    expect(screen.queryByRole("option", { name: "remote:cloud" })).not.toBeInTheDocument();
  });

  it("explains when Ollama is not running", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => json({ ...STATUS, reachable: false, message: "Ollama is not running." })));
    render(<ModelSettings client={client()} settings={settings} onChange={vi.fn()} />);
    expect(await screen.findByTestId("ollama-status")).toHaveTextContent("Ollama is not running.");
  });

  it("downloads with progress and requires confirmation before removing", async () => {
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url.includes("/pull")) return stream([{ type: "progress", status: "pulling", total: 10, completed: 5 }, { type: "done" }]);
      if (init?.method === "DELETE") return new Response(null, { status: 204 });
      return json(STATUS);
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<ModelSettings client={client()} settings={settings} onChange={vi.fn()} />);

    await userEvent.type(await screen.findByLabelText("Model to download"), "new:1{enter}");
    await waitFor(() => expect(fetchMock.mock.calls.some(([url]) => url.includes("/pull"))).toBe(true));

    await userEvent.click(await screen.findByRole("button", { name: "Remove local:1" }));
    expect(fetchMock.mock.calls.some(([, init]) => init?.method === "DELETE")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Confirm remove" }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([, init]) => init?.method === "DELETE")).toBe(true));
  });
});
