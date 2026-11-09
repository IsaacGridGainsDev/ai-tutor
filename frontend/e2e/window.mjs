import { chromium } from "@playwright/test";

const WINDOW_TITLE = "Interactive Tutor";
const TIMEOUT_MS = 30_000;

/** Connects over CDP and waits for the Tutor window, refusing to touch any other application. */
export async function connectToTutor(port) {
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const deadline = Date.now() + TIMEOUT_MS;
  while (Date.now() < deadline) {
    const pages = browser.contexts().flatMap((context) => context.pages());
    for (const page of pages) {
      if ((await page.title()) === WINDOW_TITLE) return { browser, page };
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  await browser.close();
  throw new Error(`${WINDOW_TITLE} window not found on CDP port ${port}`);
}
