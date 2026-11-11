import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import { ApiClient, ApiError } from "./api/client";
import { invokerOver } from "./api/testing";

// The app runs as it does in the desktop window, with the engine commands answered by the same
// route table as the backend calls.
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: (command: string, args?: Record<string, unknown>) =>
    command === "backend_info"
      ? Promise.resolve({ baseUrl: "http://x", token: "t" })
      : invokerOver((url, init) => fetch(url, init))(command, args),
}));

const SETTINGS = {
  llm_model: null,
  embedding_model: null,
  speech_model: null,
  voice: null,
  reading_speed: 1,
  explanation_depth: "standard",
  assessment_difficulty: "standard",
  compute_preference: "auto",
  document_storage_path: null,
};

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

describe("ApiClient", () => {
  it("sends the launch token and surfaces API errors", async () => {
    const fetchMock = vi.fn().mockResolvedValue(json({ detail: "nope" }, 401));
    vi.stubGlobal("fetch", fetchMock);
    const client = new ApiClient({ baseUrl: "http://127.0.0.1:1", token: "abc" });

    const failure = client.health();
    await expect(failure).rejects.toBeInstanceOf(ApiError);
    await expect(failure).rejects.toThrow("nope");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://127.0.0.1:1/api/health");
    expect(new Headers(init.headers).get("x-tutor-token")).toBe("abc");
  });
});

describe("App", () => {
  it("connects to the backend and saves a setting", async () => {
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url.endsWith("/health")) return json({ status: "ok", version: "0.1.0", schema_version: 1 });
      if (url.includes("/books") || url.includes("/collections")) return json([]);
      if (url.includes("/ai/status")) return json({ reachable: false, message: "down", models: [], excluded_cloud_models: [], running: [], embedding: { mode: "hashing", model: null } });
      return json(init?.method === "PUT" ? JSON.parse(init.body as string) : SETTINGS);
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<App />);

    expect(await screen.findByText(/Backend connected/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(await screen.findByLabelText("Compute preference"), "cpu");

    const put = fetchMock.mock.calls.find(([, init]) => init?.method === "PUT");
    expect(JSON.parse(put![1]!.body as string).compute_preference).toBe("cpu");
    expect(screen.getByLabelText("Compute preference")).toHaveValue("cpu");
  });
});
