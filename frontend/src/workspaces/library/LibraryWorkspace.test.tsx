import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ApiClient, type Book } from "../../api/client";
import { LibraryWorkspace } from "./LibraryWorkspace";

function book(overrides: Partial<Book> = {}): Book {
  return {
    id: 1,
    title: "Physics",
    author: "N. Newton",
    file_name: "physics.pdf",
    page_count: 12,
    status: "ready",
    status_detail: null,
    progress: 1,
    needs_ocr: false,
    created_at: "2026-11-09",
    chapter_count: 3,
    passage_count: 9,
    collection_ids: [],
    ...overrides,
  };
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status });
}

function setup(routes: (url: string, init?: RequestInit) => Response | undefined) {
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const handled = routes(url, init);
    if (handled) return handled;
    if (url.includes("/ai/status")) {
      return json({ reachable: false, message: "down", models: [], excluded_cloud_models: [], running: [], embedding: { mode: "hashing", model: null } });
    }
    if (url.includes("/collections")) return json([]);
    return json([]);
  });
  vi.stubGlobal("fetch", fetchMock);
  const client = new ApiClient({ baseUrl: "http://x", token: "t" });
  render(<LibraryWorkspace client={client} />);
  return fetchMock;
}

describe("LibraryWorkspace", () => {
  it("uploads a file as multipart without overriding the content type", async () => {
    const fetchMock = setup((_url, init) => (init?.method === "POST" ? json(book(), 201) : undefined));
    const file = new File(["%PDF-"], "new.pdf", { type: "application/pdf" });
    await userEvent.upload(screen.getByLabelText("Choose PDF files"), file);

    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === "POST")).toBe(true));
    const [url, init] = fetchMock.mock.calls.find(([, i]) => i?.method === "POST")!;
    expect(url).toBe("http://x/api/books");
    expect(init!.body).toBeInstanceOf(FormData);
    expect(new Headers(init!.headers).has("content-type")).toBe(false);
  });

  it("shows failures with their reason and requires confirmation to remove", async () => {
    const failed = book({ status: "failed", status_detail: "This PDF is password protected." });
    const fetchMock = setup((url, init) => {
      if (init?.method === "DELETE") return new Response(null, { status: 204 });
      if (url.includes("/books")) return json([failed]);
      return undefined;
    });

    expect(await screen.findByText("This PDF is password protected.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(fetchMock.mock.calls.some(([, i]) => i?.method === "DELETE")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Confirm remove" }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([, i]) => i?.method === "DELETE")).toBe(true));
  });

  it("does not let an unfinished book be opened and shows its progress", async () => {
    setup((url) => (url.includes("/books") ? json([book({ status: "ocr", progress: 0.4, status_detail: "Recognising" })]) : undefined));
    expect(await screen.findByRole("progressbar")).toHaveAttribute("aria-valuenow", "40");
    expect(screen.queryByRole("button", { name: "Physics" })).not.toBeInTheDocument();
  });

  it("renders search snippets as text, not HTML", async () => {
    setup((url) => {
      if (url.includes("/search")) {
        return json([
          {
            passage_id: 1, chapter_id: 1, section_id: 1, chapter_title: "C", section_title: "S",
            snippet: "<img src=x> [[energy]] here", start_page: 4, end_page: 5,
          },
        ]);
      }
      if (url.includes("/structure")) return json([]);
      if (url.includes("/books")) return json([book()]);
      return undefined;
    });
    await userEvent.click(await screen.findByRole("button", { name: "Physics" }));
    await userEvent.type(screen.getByLabelText("Search inside this book"), "energy{enter}");

    expect(await screen.findByText("energy")).toHaveProperty("tagName", "MARK");
    expect(screen.getByText(/pp\. 4–5/)).toBeInTheDocument();
    expect(document.querySelector("img")).toBeNull();
  });
});
