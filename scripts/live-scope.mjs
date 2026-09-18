// Live check of the scope rules against the REAL desktop app (see live-rework.mjs for how to start it):
// an emptied selection must never become a whole-document request, and an explicit
// whole-document rework must not delete sections. Uses the provider that is configured.
import { chromium } from "@playwright/test";

const log = (...parts) => console.log(new Date().toISOString().slice(11, 19), ...parts);
const fail = async message => { log("FAILED:", message); await browser.close(); process.exit(1); };
const browser = await chromium.connectOverCDP("http://127.0.0.1:9223");
const page = browser.contexts().flatMap(context => context.pages()).find(p => p.url().includes("localhost:5173"));
if (!page) throw new Error("ArchGen is not running with --remote-debugging-port=9223.");

if (await page.locator("h2.section-title").count() === 0) {
  await page.getByRole("button", { name: "Slovensky", exact: true }).click();
  await page.getByRole("button", { name: /^C4 Model/ }).click();
  await page.locator("h2.section-title").first().waitFor();
}
const titles = await page.locator("h2.section-title").allInnerTexts();
const generate = page.getByRole("button", { name: "Generate", exact: true });
const prompt = page.locator("textarea.prompt-input");
const first = page.getByRole("checkbox", { name: `Select section 1: ${titles[0]} for AI`, exact: true });

// 1. Rework clicked earlier, then a selection that disappears again: nothing may be sent.
await page.getByRole("button", { name: "Rework", exact: true }).click();
log("armed:", JSON.stringify(await page.locator(".selection-summary").innerText()));
await first.check();
await prompt.fill("Toto sa nesmie odoslať.");
await first.uncheck();
const panel = await page.locator(".prompt-panel").innerText();
log("after untick:", JSON.stringify(panel.slice(0, 260)));
if (!(await generate.isDisabled())) await fail("Generate is enabled although nothing is selected and nothing is armed.");
if (!panel.includes("Tick blocks, or click Rework")) await fail("The hint for an empty scope is missing.");
await prompt.press("Enter");
await page.waitForTimeout(1500);
if (await page.locator(".project-status", { hasText: /queued|running/ }).count()) await fail("Enter created a job with an empty scope.");
log("OK: an emptied selection sends nothing (button and Enter).");

// 2. An explicit whole-document rework keeps every section.
await page.getByRole("button", { name: "Rework", exact: true }).click();
log("consent:", JSON.stringify(await page.locator("#rework-consent").innerText()));
await prompt.fill("Do každej sekcie napíš presne jednu vetu o e-shope. Žiadnu sekciu nepridávaj ani neodstraňuj.");
const started = Date.now();
await generate.click();
const review = page.getByRole("button", { name: "Review changes", exact: true });
const failed = page.locator(".project-status", { hasText: /failed|interrupted/ });
await Promise.race([review.waitFor({ timeout: 900_000 }), failed.first().waitFor({ timeout: 900_000 })]);
log(`finished after ${Math.round((Date.now() - started) / 1000)} s`);
if (await failed.count()) await fail(await failed.first().innerText());
await review.click();
const modal = page.getByRole("dialog");
const count = kind => modal.locator(`[data-kind="${kind}"]`).count();
log("changed:", await count("changed"), "added:", await count("added"), "removed:", await count("removed"));
log("review:\n" + (await modal.innerText()).slice(0, 1500));
if (await count("removed")) await fail("A whole-document rework removed sections.");
await modal.getByRole("button", { name: "Accept changes", exact: true }).click();
await page.waitForTimeout(800);
const after = await page.locator("h2.section-title").allInnerTexts();
log("sections after accept:", after);
if (after.length < titles.length) await fail("Sections were lost.");
if (await generate.isEnabled() && (await page.locator(".selection-summary").innerText()).includes("Whole document")) await fail("Document scope stayed armed after the request.");
log("OK: whole-document rework kept every section, and the arm was dropped afterwards.");
await browser.close();
