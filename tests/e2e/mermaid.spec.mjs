import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { installIpcMock } from "./ipc-mock.mjs";

const aiStatus = JSON.parse(readFileSync(new URL("../fixtures/ai-status.json", import.meta.url), "utf8"));

// UI integration tests mock IPC only; mermaid.ts itself never goes through IPC (it
// renders entirely in-page), which is exactly the property these tests check.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(installIpcMock, { aiStatus });
  await page.goto("/");
});

async function openMermaidFixture(page, content) {
  await page.evaluate((fixtureContent) => {
    window.loadFixture = { schemaVersion: 1, id: "local-project", name: "Mermaid diagrams", revision: 0, activeDocumentId: "doc", documents: [
      { id: "doc", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "section", title: "Local model", content: "", diagrams: [
          { id: "diagram", content: fixtureContent, diagram_type: "flow", format: "mermaid" },
        ] },
      ] },
    ] };
  }, content);
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

function decodeSvg(dataUrl) {
  return decodeURIComponent(dataUrl.split(",").slice(1).join(","));
}

test("Mermaid renders automatically offline, sanitized, with readable labels and no upload", async ({ page }) => {
  const remote = [];
  page.on("request", (r) => { if (r.url().startsWith("http") && !r.url().startsWith("http://127.0.0.1")) remote.push(r.url()); });
  await openMermaidFixture(page, "flowchart TD\n  Start[StartNode] --> Decide{ReadyCheck}\n  Decide -->|YesBranch| Done[AllDone]\n  Decide -->|NoBranch| Start");

  const preview = page.getByRole("img", { name: "Mermaid Diagram" });
  await expect(preview).toBeVisible({ timeout: 10000 });
  const svg = decodeSvg(await preview.getAttribute("src"));
  // Readable labels: the node text survived local-svg.ts (foreignObject is forbidden
  // there, so this only holds because mermaid.ts renders labels as plain SVG <text>).
  expect(svg).toContain("StartNode");
  expect(svg).toContain("ReadyCheck");
  expect(svg).toContain("AllDone");
  expect(svg).toContain("YesBranch");
  // Never a live DOM element, no active/external content, nothing left to run or fetch.
  expect(svg).not.toMatch(/<script|foreignobject|<a[\s>]|javascript:|<image[\s>]|<use[\s>]/i);
  expect(remote).toEqual([]);
  expect(await page.evaluate(() => window.calls.some((c) => c.command === "render_local_diagram"))).toBe(false);
});

test("an invalid Mermaid source shows an error and keeps the source visible, without crashing", async ({ page }) => {
  await openMermaidFixture(page, "this is not a mermaid diagram {{{ ???");

  await expect(page.locator(".puml-error[role=alert]")).toBeVisible({ timeout: 10000 });
  await expect(page.locator(".puml-error[role=alert]")).toContainText("Invalid Mermaid diagram");
  await expect(page.locator(".puml-code")).toContainText("this is not a mermaid diagram");
  await expect(page.getByRole("img", { name: "Mermaid Diagram" })).toHaveCount(0);

  // The Code toggle still works, and switching it does not disturb the error view; the
  // rest of the app (a completely unrelated action) still works — no crash.
  const preview = page.locator(".puml-preview");
  await preview.getByRole("button", { name: "Code", exact: true }).click();
  await expect(page.locator(".puml-code")).toContainText("this is not a mermaid diagram");
  await preview.getByRole("button", { name: "Diagram", exact: true }).click();
  await expect(page.locator(".puml-error[role=alert]")).toBeVisible();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
});

test("editing Mermaid content re-renders it, and a stale render never overwrites newer content", async ({ page }) => {
  // Short, space-free labels: long ones get word-wrapped by Mermaid itself into
  // several <tspan>s, which would make an exact-substring check fragile for reasons
  // that have nothing to do with the staleness guard this test is about.
  await openMermaidFixture(page, "flowchart TD\n  A[StaleOld] --> B[StaleOldB]");
  // Deliberately do not wait for the first (cold-start, dynamic-import) render to
  // settle before asking to replace the diagram, so its late result races the new one.
  await page.locator("textarea.prompt-input").fill("update the diagram with a new label");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  await page.evaluate(() => window.resolveGeneration({ summary: "Relabelled.", diagram: { format: "mermaid", content: "flowchart TD\n  X[FreshNew] --> Y[FreshNewB]" } }));
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  await page.getByRole("button", { name: "Accept changes", exact: true }).click();

  const preview = page.getByRole("img", { name: "Mermaid Diagram" });
  await expect(preview).toBeVisible({ timeout: 10000 });
  // Give any earlier, now-superseded render every chance to resolve late and (wrongly)
  // overwrite the newer one before asserting on the final, settled state.
  await page.waitForTimeout(1000);
  const svg = decodeSvg(await preview.getAttribute("src"));
  expect(svg).toContain("FreshNew");
  expect(svg).not.toContain("StaleOld");
});
