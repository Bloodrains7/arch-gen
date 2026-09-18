// Live smoke test of "AI rework" against the REAL desktop app: real Tauri IPC, real provider.
// The mocked e2e tests cannot see a misnamed IPC argument or a provider that changed its output.
//
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9223"; npm run tauri dev
//   node scripts/live-rework.mjs "Ollama (local)" 1 qwen3.8:27b
//   node scripts/live-rework.mjs "Claude CLI" 2          # costs a little of your Claude quota
//   node scripts/live-rework.mjs "Codex CLI" 3           # costs a little of your Codex quota
//
// Arguments: provider label as shown in AI settings, 1-based section number, optional model.
// It changes the provider in your AI settings; set it back in the dialog afterwards.
// SHOTS=<folder> saves screenshots. Exit code 1 = the job failed (the sentence is printed).
import { chromium } from "@playwright/test";

const [provider = "Ollama (local)", sectionNumber = "1", model = ""] = process.argv.slice(2);
const shots = process.env.SHOTS;
const log = (...parts) => console.log(new Date().toISOString().slice(11, 19), ...parts);

const browser = await chromium.connectOverCDP("http://127.0.0.1:9223");
const page = browser.contexts().flatMap(context => context.pages()).find(p => p.url().includes("localhost:5173"));
if (!page) throw new Error("ArchGen is not running with --remote-debugging-port=9223.");
const invoke = (command, args = {}) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [command, args]);
const shot = async name => { if (shots) await page.screenshot({ path: `${shots}/${name}.png` }); };

if (await page.locator("h2.section-title").count() === 0) {
  await page.getByRole("button", { name: "Slovensky", exact: true }).click();
  await page.getByRole("button", { name: /^C4 Model/ }).click();
  await page.locator("h2.section-title").first().waitFor();
}
const titles = await page.locator("h2.section-title").allInnerTexts();
const n = Number(sectionNumber);
for (const box of await page.getByRole("checkbox", { name: /^Select section/ }).all()) if (await box.isChecked()) await box.uncheck();
await page.getByRole("checkbox", { name: `Select section ${n}: ${titles[n - 1]} for AI`, exact: true }).check();

await page.getByRole("button", { name: "AI settings", exact: true }).click();
const settings = page.getByRole("dialog", { name: "AI settings" });
await settings.getByRole("radio", { name: provider, exact: true }).check();
await page.waitForTimeout(500);
if (model) {
  const select = settings.locator("select");
  if (await select.count()) await select.selectOption(model);
  else {
    await settings.locator("label.field", { hasText: "Model" }).locator("input").fill(model);
    await page.keyboard.press("Tab");
  }
  await page.waitForTimeout(500);
}
const status = await invoke("ai_status");
log("configured:", JSON.stringify(status.providers.find(p => p.id === status.provider)));
await shot(`settings-${status.provider}`);
await settings.getByRole("button", { name: "Close", exact: true }).click();
log("consent:", JSON.stringify(await page.locator("#rework-consent").innerText()));

await page.locator("textarea.prompt-input").fill(`Napíš presne dve vety o sekcii „${titles[n - 1]}“ pre e-shop so zákazníkom, platobnou bránou a skladom.`);
const started = Date.now();
await page.getByRole("button", { name: "Generate", exact: true }).click();
const review = page.getByRole("button", { name: "Review changes", exact: true });
const failed = page.locator(".project-status", { hasText: /failed|interrupted/ });
await Promise.race([review.waitFor({ timeout: 900_000 }), failed.first().waitFor({ timeout: 900_000 })]);
log(`finished after ${Math.round((Date.now() - started) / 1000)} s`);

if (await failed.count()) {
  log("JOB FAILED:", await failed.first().innerText());
  await shot(`failed-${status.provider}`);
  await failed.first().getByRole("button", { name: "Dismiss job" }).click().catch(() => {});
  await browser.close();
  process.exit(1);
}
await review.click();
const modal = page.getByRole("dialog");
const count = kind => modal.locator(`[data-kind="${kind}"]`).count();
log("changed:", await count("changed"), "added:", await count("added"), "removed:", await count("removed"));
log("review:\n" + (await modal.innerText()).slice(0, 1800));
await shot(`review-${status.provider}`);
if (await count("changed") !== 1 || await count("added") || await count("removed")) {
  log("UNEXPECTED: exactly the one selected section should have changed.");
  await browser.close();
  process.exit(1);
}
await modal.getByRole("button", { name: "Accept changes", exact: true }).click();
await page.waitForTimeout(800);
log("after accept:", await page.getByRole("status").first().innerText());
await browser.close();
