// Drives the real Tauri window through a full library workflow.
// Usage: node e2e/library.mjs <cdp-port> <textbook.pdf> <scanned.pdf> [screenshot-path]
// Launch a debug build with TUTOR_E2E_CDP_PORT=<cdp-port> first, on a port confirmed free.
import { connectToTutor } from "./window.mjs";

const [port, textbook, scanned, screenshot] = process.argv.slice(2);
if (!scanned) throw new Error("Usage: library.mjs <cdp-port> <textbook.pdf> <scanned.pdf> [screenshot]");

const { browser, page } = await connectToTutor(port);
try {
  await page.getByTestId("backend-status").getByText("Backend connected").waitFor({ timeout: 60_000 });
  const started = Date.now();
  await page.getByLabel("Choose PDF files").setInputFiles([textbook, scanned]);

  const statuses = page.getByTestId("book-status");
  await page.waitForFunction(() => {
    const all = [...document.querySelectorAll('[data-testid="book-status"]')];
    return all.length === 2 && all.every((el) => ["Ready", "Failed"].includes(el.textContent));
  }, undefined, { timeout: 300_000 });
  console.log("statuses:", await statuses.allInnerTexts(), `${Date.now() - started} ms`);

  const row = page.getByTestId("book-row").filter({ hasText: "thinkpython2.pdf" });
  console.log("book row:", (await row.innerText()).replace(/\s+/g, " "));
  await row.getByRole("button", { name: "Think Python" }).click();
  await page.getByRole("button", { name: /^Functions/ }).click();
  await page.getByRole("button", { name: /^Math functions/ }).click();
  await page.getByRole("heading", { name: "Math functions" }).waitFor();
  const passage = page.getByTestId("passage").first();
  await passage.waitFor();
  console.log("section passage:", (await passage.innerText()).replace(/\s+/g, " ").slice(0, 160));

  const search = page.getByLabel("Search inside this book");
  await search.fill("recursion base case");
  await search.press("Enter");
  const result = page.getByRole("region", { name: "Search results" }).getByRole("button").first();
  await result.waitFor();
  console.log("search hit:", (await result.innerText()).replace(/\s+/g, " ").slice(0, 200));
  await result.click();
  await page.getByTestId("passage").first().waitFor();
  console.log("opened section:", await page.getByRole("article", { name: "Passages" }).getByRole("heading").first().innerText());
  if (screenshot) await page.screenshot({ path: screenshot });
} finally {
  await browser.close();
}
