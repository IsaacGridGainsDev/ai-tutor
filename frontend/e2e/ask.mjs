// Asks the real local model questions through the real Tauri window.
// Usage: node e2e/ask.mjs <cdp-port> <textbook.pdf> [screenshot-path]
// Launch a debug build with TUTOR_E2E_CDP_PORT=<cdp-port> first, on a port confirmed free.
import { connectToTutor } from "./window.mjs";

const [port, textbook, screenshot] = process.argv.slice(2);
if (!textbook) throw new Error("Usage: ask.mjs <cdp-port> <textbook.pdf> [screenshot]");

const ANSWER_TIMEOUT_MS = 400_000;
const { browser, page } = await connectToTutor(port);
const flat = (text) => text.replace(/\s+/g, " ");

async function ask(question) {
  await page.getByLabel("Your question").fill(question);
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  const started = Date.now();
  await page.getByRole("button", { name: "Stop" }).waitFor();
  await page.getByRole("button", { name: "Ask", exact: true }).waitFor({ timeout: ANSWER_TIMEOUT_MS });
  console.log(`\nQ: ${question} (${Math.round((Date.now() - started) / 1000)}s)`);
  const alert = page.getByRole("alert");
  if (await alert.count()) console.log("ALERT:", await alert.allInnerTexts());
  console.log("segments:", await page.getByTestId("segment-kind").allInnerTexts());
  console.log("answer:", flat(await page.getByTestId("answer").innerText()).slice(0, 500));
}

try {
  await page.getByTestId("backend-status").getByText("Backend connected").waitFor({ timeout: 60_000 });

  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByTestId("ollama-status").waitFor();
  console.log("ollama:", await page.getByTestId("ollama-status").innerText());
  console.log("installed:", (await page.getByTestId("installed-model").allInnerTexts()).map(flat));
  console.log("excluded:", await page.getByTestId("excluded-cloud").innerText());
  console.log("embedding:", await page.getByTestId("embedding-status").innerText());

  await page.getByRole("button", { name: "My Library" }).click();
  await page.getByLabel("Choose PDF files").setInputFiles([textbook]);
  const row = page.getByTestId("book-row").filter({ hasText: "thinkpython2.pdf" });
  await row.getByTestId("book-status").getByText("Ready").waitFor({ timeout: 120_000 });
  await row.getByRole("button", { name: "Think Python" }).click();

  await page.getByRole("tab", { name: "concepts" }).click();
  await page.getByTestId("concept").first().waitFor();
  console.log("first concept:", flat(await page.getByTestId("concept").first().innerText()).slice(0, 200));

  await page.getByRole("tab", { name: "ask" }).click();
  await page.getByTestId("model-status").getByText("Local model").waitFor();
  console.log("model status:", await page.getByTestId("model-status").innerText());

  // Stop must end a real generation and leave the panel usable.
  await page.getByLabel("Your question").fill("Explain how recursion works in detail");
  await page.getByRole("button", { name: "Ask", exact: true }).click();
  await page.getByTestId("ask-stage").getByText("writing an answer").waitFor({ timeout: 60_000 });
  await page.getByRole("button", { name: "Stop" }).click();
  await page.getByRole("button", { name: "Ask", exact: true }).waitFor({ timeout: 15_000 });
  console.log("stop: returned to idle, alerts:", await page.getByRole("alert").count());

  await ask("What is a base case in recursion?");
  if (screenshot) await page.screenshot({ path: screenshot });

  const citation = page.getByTestId("answer").getByRole("button").first();
  if (await citation.count()) {
    console.log("citation:", await citation.innerText());
    await citation.click();
    await page.getByTestId("passage").first().waitFor();
    console.log("opened section:", await page.getByRole("article", { name: "Passages" }).getByRole("heading").first().innerText());
    await page.getByRole("tab", { name: "ask" }).click();
  }

  await ask("Who won the football World Cup in 1998?");
} finally {
  await browser.close();
}
