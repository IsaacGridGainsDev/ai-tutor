// Drives the real Tauri window through WebView2's remote debugging port.
// Usage: node e2e/desktop-shell.mjs <cdp-port> [screenshot-path]
// Launch a debug build with TUTOR_E2E_CDP_PORT=<cdp-port> first, on a port confirmed free:
// other Chromium-based apps on the machine may also expose one.
import { connectToTutor } from "./window.mjs";

const [port, screenshot] = process.argv.slice(2);
if (!port) throw new Error("A CDP port is required");

const { browser, page } = await connectToTutor(port);
try {
  const status = page.getByTestId("backend-status");
  await status.getByText("Backend connected").waitFor({ timeout: 60_000 });
  console.log("status:", await status.innerText());
  console.log("origin:", await page.evaluate(() => location.origin));

  await page.getByRole("button", { name: "Settings" }).click();
  const compute = page.getByLabel("Compute preference");
  await compute.selectOption("cpu");
  await page.waitForTimeout(300);
  await page.getByRole("button", { name: "My Library" }).click();
  await page.getByRole("button", { name: "Settings" }).click();
  await page.waitForTimeout(300);
  console.log("persisted compute preference:", await compute.inputValue());
  await compute.selectOption("auto");
  await page.waitForTimeout(300);
  if (screenshot) await page.screenshot({ path: screenshot });
} finally {
  await browser.close();
}
