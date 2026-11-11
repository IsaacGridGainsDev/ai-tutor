import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ApiClient, type Book, type SearchHit, type SearchResponse } from "../../api/client";
import { invokerOver } from "../../api/testing";
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

function hit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    passage_id: 1, book_id: 1, chapter_id: 1, section_id: 1, chapter_title: "Cells", section_title: "Energy",
    snippet: "energy here", start_page: 4, end_page: 5, from_ocr: false,
    ...overrides,
  };
}

function found(hits: SearchHit[], title = "Physics", bookId = 1): SearchResponse {
  return {
    query: "energy", match_mode: "all", total: hits.length, concepts: [],
    groups: [{ book_id: bookId, book_title: title, total: hits.length, hits }],
  };
}

const STRUCTURE = [
  {
    id: 5, title: "Cells", start_page: 1, end_page: 9, detection_method: "outline",
    sections: [
      { id: 7, title: "Membranes", start_page: 1, end_page: 2, passage_count: 2 },
      { id: 8, title: "Energy", start_page: 3, end_page: 5, passage_count: 2 },
    ],
  },
];

const CONCEPT = {
  id: 3, name: "Osmosis", definition: "Water crossing a membrane.", book_id: 1, book_title: "Physics",
  passage_id: 1, section_id: 1, start_page: 4,
};

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
  const client = new ApiClient({ baseUrl: "http://x", token: "t" }, invokerOver(fetchMock));
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

  it("searches inside a book as you type and renders snippets as text, not HTML", async () => {
    const fetchMock = setup((url) => {
      if (url.includes("/search/recent")) return json([]);
      if (url.includes("/search")) return json(found([hit({ snippet: "<img src=x> \u0002energy\u0003 here" })]));
      if (url.includes("/structure")) return json([]);
      if (url.includes("/books")) return json([book()]);
      return undefined;
    });
    await userEvent.click(await screen.findByRole("button", { name: "Physics" }));
    await userEvent.type(screen.getByLabelText("Search inside this book"), "ener");

    expect(await screen.findByText("energy")).toHaveProperty("tagName", "MARK");
    expect(screen.getByText(/pp\. 4–5/)).toBeInTheDocument();
    expect(document.querySelector("img")).toBeNull();
    const searched = new URL(fetchMock.mock.calls.map(([url]) => url).find((url) => url.includes("/search?"))!);
    expect(Object.fromEntries(searched.searchParams)).toEqual({ q: "ener", scope: "book", book_id: "1", live: "true" });
  });

  it("searches every book, opens a result at its passage and steps through the matches", async () => {
    const fetchMock = setup((url, init) => {
      if (url.includes("/search/recent")) return init?.method === "POST" ? new Response(null, { status: 204 }) : json([]);
      if (url.includes("/search")) {
        return json(found([hit({ book_id: 2, passage_id: 21, section_id: 8, chapter_id: 5, from_ocr: true })], "Biology", 2));
      }
      if (url.includes("/structure")) return json(STRUCTURE);
      if (url.includes("/sections/8/passages")) {
        return json([
          { id: 20, section_id: 8, ordinal: 0, text: "Cells need energy.", highlighted: "Cells need \u0002energy\u0003.", start_page: 3, end_page: 3 },
          { id: 21, section_id: 8, ordinal: 1, text: "Energy is stored.\n\nEnergy is released.", highlighted: "\u0002Energy\u0003 is stored.\n\n\u0002Energy\u0003 is released.", start_page: 4, end_page: 4 },
        ]);
      }
      if (url.includes("/books")) return json([book(), book({ id: 2, title: "Biology" })]);
      return undefined;
    });

    await userEvent.type(await screen.findByLabelText("Search all books"), "energy");
    const group = await screen.findByRole("group", { name: "Biology" });
    expect(within(group).getByText(/Cells › Energy · pp\. 4–5 · scanned/)).toBeInTheDocument();
    await userEvent.click(within(group).getByRole("button"));

    // The other book opens at the section, with the match in the chosen passage current.
    expect(await screen.findByRole("heading", { name: "Biology" })).toBeInTheDocument();
    expect(await screen.findByText("Match 2 of 3 for “energy”")).toBeInTheDocument();
    expect([...document.querySelectorAll("mark")].map((node) => node.textContent)).toEqual(["energy", "Energy", "Energy"]);
    await userEvent.click(screen.getByRole("button", { name: "Next ›" }));
    expect(screen.getByText("Match 3 of 3 for “energy”")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Next ›" }));
    expect(screen.getByText("Match 1 of 3 for “energy”")).toBeInTheDocument();

    const passages = fetchMock.mock.calls.map(([url]) => url).find((url) => url.includes("/sections/8/passages"))!;
    expect(new URL(passages).searchParams.get("q")).toBe("energy");
    const remembered = fetchMock.mock.calls.find(([url, init]) => url.includes("/search/recent") && init?.method === "POST")!;
    expect(JSON.parse(remembered[1]!.body as string)).toEqual({ query: "energy", scope: "all", book_id: null, result_title: "Energy" });
  });

  it("offers recent searches and says when only some words matched", async () => {
    setup((url) => {
      if (url.includes("/search/recent")) {
        return json([{ id: 1, query: "osmosis membrane", scope: "all", book_id: null, book_title: null, result_title: "Cells", created_at: "" }]);
      }
      if (url.includes("/search")) return json({ ...found([hit()]), match_mode: "any", concepts: [CONCEPT] });
      return undefined;
    });
    await userEvent.click(await screen.findByLabelText("Search all books"));
    await userEvent.click(await screen.findByRole("button", { name: /osmosis membrane/ }));

    expect(screen.getByLabelText("Search all books")).toHaveValue("osmosis membrane");
    expect(await screen.findByText(/no passage has every word/)).toBeInTheDocument();
    expect(within(screen.getByRole("group", { name: "Matching concepts" })).getByRole("button", { name: /Osmosis/ })).toBeInTheDocument();
  });
});
