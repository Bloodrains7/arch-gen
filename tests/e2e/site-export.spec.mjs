import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { installIpcMock } from "./ipc-mock.mjs";

const aiStatus = JSON.parse(readFileSync(new URL("../fixtures/ai-status.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  await page.addInitScript(installIpcMock, { aiStatus });
  await page.goto("/");
});

async function openTwoDocumentFixture(page) {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Billing platform", revision: 0, activeDocumentId: "doc1", documents: [
      { id: "doc1", name: "Billing", template: "arc42", language: "en", revision: 0, sections: [
        { id: "s1", title: "Context", content: "Users and the bank.", diagrams: [
          { id: "g1", content: "@startuml\nA -> B\n@enduml", diagram_type: "sequence", format: "plantuml" },
          { id: "g2", content: "graph TD\nA-->B", diagram_type: "flow", format: "mermaid" },
        ] },
      ] },
      { id: "doc2", name: "Ops", template: "custom", language: "en", revision: 0, sections: [
        { id: "s2", title: "Overview", content: "Runbook.", diagrams: [] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

test("Export site renders PlantUML locally, keeps Mermaid as source and reports the result", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.getByRole("button", { name: "Export site…", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("Exported 2 documents to fixture-project");
  await expect(page.locator(".project-status").first()).toContainText("1 diagram as SVG");
  await expect(page.locator(".project-status").first()).toContainText("1 diagram as source");
  await expect(page.locator(".project-status").first()).not.toContainText("unsaved edits");

  const call = await page.evaluate(() => window.calls.find(c => c.command === "export_site"));
  expect(call.args.path).toBe("fixture-project");
  const files = Object.fromEntries(call.args.files.map(f => [f.path, f.content]));
  expect(Object.keys(files).sort()).toEqual([
    "billing.md", "images/billing/context-1.svg", "index.md", "mkdocs.yml", "ops.md", "toc.yml",
  ]);
  expect(files["billing.md"]).toContain("![sequence diagram](images/billing/context-1.svg)");
  expect(files["billing.md"]).toContain("```mermaid\ngraph TD\nA-->B\n```");
  expect(files["ops.md"]).not.toContain("images/");
  expect(files["index.md"]).toContain("[Billing](billing.md)");
  expect(files["index.md"]).toContain("[Ops](ops.md)");
  expect(files["toc.yml"]).toBe('- name: "Overview"\n  href: index.md\n- name: "Billing"\n  href: billing.md\n- name: "Ops"\n  href: ops.md\n');
  // The PlantUML SVG is the same one the local-render tests assert is sanitized (no script/foreignObject/url()).
  expect(files["images/billing/context-1.svg"]).toContain("Local preview");
  expect(files["images/billing/context-1.svg"]).not.toMatch(/script|foreignObject|<image|example\.invalid|url\(/i);

  // render_local_diagram was only asked for the PlantUML diagram, never the Mermaid one.
  const renderCalls = await page.evaluate(() => window.calls.filter(c => c.command === "render_local_diagram"));
  expect(renderCalls).toHaveLength(1);
  expect(renderCalls[0].args.content).toBe("@startuml\nA -> B\n@enduml");
});

test("unsaved edits are included in the export and said so in the status line", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.getByLabel("Project name").fill("Billing platform (renamed)");
  await page.getByRole("button", { name: "Export site…", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("unsaved edits included");
  const call = await page.evaluate(() => window.calls.find(c => c.command === "export_site"));
  const index = call.args.files.find(f => f.path === "index.md").content;
  expect(index).toContain("# Billing platform (renamed)");
});

test("a local PlantUML render failure falls back to fenced source instead of failing the export", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.evaluate(() => { window.renderFailure = true; });
  await page.getByRole("button", { name: "Export site…", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("2 diagrams as source");
  const call = await page.evaluate(() => window.calls.find(c => c.command === "export_site"));
  expect(call.args.files.some(f => f.path.startsWith("images/"))).toBe(false);
  const billing = call.args.files.find(f => f.path === "billing.md").content;
  expect(billing).toContain("```plantuml\n@startuml\nA -> B\n@enduml\n```");
});

test("export_site failure is reported on the status line and does not crash the app", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.evaluate(() => { window.exportSiteFailure = "The chosen folder is not empty."; });
  await page.getByRole("button", { name: "Export site…", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("Site export failed");
  await expect(page.locator(".project-status").first()).toContainText("not empty");
  // The app is still usable afterwards.
  await expect(page.getByRole("button", { name: "Export site…", exact: true })).toBeEnabled();
});

test("cancelling the folder picker exports nothing and leaves the status line untouched", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.evaluate(() => { window.directoryPickResult = null; });
  await page.getByRole("button", { name: "Export site…", exact: true }).click();
  // Give the (short-circuited) handler a turn to run, then confirm nothing was sent.
  await expect.poll(() => page.evaluate(() => window.calls.some(c => c.command === "plugin:dialog|open"))).toBe(true);
  expect(await page.evaluate(() => window.calls.some(c => c.command === "export_site" || c.command === "render_local_diagram"))).toBe(false);
  await expect(page.getByRole("button", { name: "Export site…", exact: true })).toBeEnabled();
});
