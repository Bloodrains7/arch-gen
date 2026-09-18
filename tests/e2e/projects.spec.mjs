import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { installIpcMock } from "./ipc-mock.mjs";

const aiStatus = JSON.parse(readFileSync(new URL("../fixtures/ai-status.json", import.meta.url), "utf8"));

// UI integration tests mock IPC only. Real SQLite persistence is tested in Rust.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(installIpcMock, { aiStatus });
  await page.goto("/");
});

async function startGeneration(page) {
  await page.getByRole("button", { name: "Docs", exact: true }).click();
  await page.locator("textarea.prompt-input").fill("Document the payment system");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
}

async function finishGeneration(page) {
  await page.evaluate(() => window.resolveGeneration({ sections: [{ title: "Generated overview", content: "Generated content", diagrams: [] }] }));
}

async function acceptChanges(page) {
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  await page.getByRole("button", { name: "Accept changes", exact: true }).click();
}

async function openDiagramFixture(page) {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "local-project", name: "Local diagrams", revision: 0, activeDocumentId: "doc", documents: [
      { id: "doc", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "section", title: "Local model", content: "", diagrams: [
          { id: "diagram", content: "@startuml\nA -> B: hello\n@enduml", diagram_type: "sequence", format: "plantuml" },
        ] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

test("local render sanitizes SVG and never uploads diagram source", async ({ page }) => {
  await openDiagramFixture(page);
  const remote = [];
  page.on("request", r => { if (r.url().startsWith("http") && !r.url().startsWith("http://127.0.0.1")) remote.push(r.url()); });
  await page.getByRole("button", { name: "Render locally (no upload)" }).click();
  const preview = page.getByRole("img", { name: "PlantUML Diagram" });
  await expect(preview).toBeVisible();
  const svg = decodeURIComponent((await preview.getAttribute("src")).split(",").slice(1).join(","));
  expect(svg).toContain("Local preview");
  expect(svg).not.toMatch(/script|foreignObject|<image|example\.invalid|url\(/i);
  expect(remote).toEqual([]);
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project"))?.documents[0].sections[0].diagrams[0].content)).toBe("@startuml\nA -> B: hello\n@enduml");
});

test("local render failure never falls back to public renderer", async ({ page }) => {
  await openDiagramFixture(page);
  await page.evaluate(() => { window.renderFailure = true; });
  await page.getByRole("button", { name: "Render locally (no upload)" }).click();
  await expect(page.locator(".puml-error[role=alert]")).toContainText("No data was sent remotely");
  await expect(page.locator('.puml-preview img')).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Render using public PlantUML server (sends diagram content)" })).toBeVisible();
});

test("ready generation survives reload and reopening its saved project", async ({ page }) => {
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  await startGeneration(page);
  await finishGeneration(page);
  await expect(page.getByRole("button", { name: "Review changes", exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await acceptChanges(page);
  await expect(page.locator("h2.section-title")).toHaveText("Generated overview");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("test-generation-jobs"))[0].status)).toBe("accepted");
});

test("cancelled generation ignores late AI output", async ({ page }) => {
  await startGeneration(page);
  await page.getByRole("button", { name: "Cancel generation", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-generation-jobs"))[0].status)).toBe("cancelled");
  await finishGeneration(page);
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Review changes", exact: true })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
});

test("journal failure preserves visible edits and retry enables saving", async ({ page }) => {
  await page.getByLabel("Project name").waitFor();
  await page.evaluate(() => { window.failNextEdit = true; });
  await page.getByLabel("Project name").fill("Preserved after failure");
  await expect(page.getByRole("button", { name: "Retry synchronization" })).toBeVisible();
  await expect(page.getByLabel("Project name")).toHaveValue("Preserved after failure");
  await page.getByRole("button", { name: "Retry synchronization" }).click();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("Preserved after failure");
});

test("save and reopen restores names, all documents and content", async ({ page }) => {
  await page.getByLabel("Project name").fill("Billing architecture");
  await page.getByRole("button", { name: "Custom Your own template structure" }).click();
  await page.getByTitle("New document").click();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("Billing architecture");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await page.getByRole("tab").first().click();
  await expect(page.locator("h2.section-title")).toHaveText("Overview");
});

test("generation updates its original tab after user switches", async ({ page }) => {
  await startGeneration(page);
  await page.getByTitle("New document").click();
  await finishGeneration(page);
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
  await acceptChanges(page);
  await page.getByRole("tab").first().click();
  await expect(page.locator("h2.section-title")).toHaveText("Generated overview");
});

test("concurrent document edit preserves work and offers recovery", async ({ page }) => {
  await startGeneration(page);
  await page.getByRole("button", { name: "Slovensky", exact: true }).click();
  await finishGeneration(page);
  await page.getByRole("button", { name: "Recover as new document" }).click();
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(page.locator("h2.section-title")).toHaveText("Generated overview");
  await page.getByRole("tab").first().click();
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
});

test("failed project open and cancelled discard preserve current work", async ({ page }) => {
  await page.getByLabel("Project name").fill("Keep this project");
  await page.evaluate(() => { window.confirmResult = false; });
  await page.getByRole("button", { name: "New project", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("Keep this project");
  await page.evaluate(() => { window.confirmResult = true; window.loadFixture = { schemaVersion: 99 }; });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Unsupported project version");
  await expect(page.getByLabel("Project name")).toHaveValue("Keep this project");
});

test("untrusted Markdown is sanitized and diagrams do not send data automatically", async ({ page }) => {
  const external = [];
  page.on("request", r => { if (!r.url().startsWith("http://127.0.0.1")) external.push(r.url()); });
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Imported", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Imported doc", revision: 0, template: "custom", language: "en", sections: [
        { id: "s", title: "Safe section", content: '<img src="https://example.invalid/secret" onerror="window.injected=true"><a href="javascript:window.injected=true">Bad link</a>',
          diagrams: [{ id: "g", content: "@startuml\nclass Customer\n@enduml", diagram_type: "class", format: "plantuml" }] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("button", { name: "Preview", exact: true }).first().click();
  await expect(page.locator(".md-preview img, .md-preview [href^='javascript:']")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Render using public PlantUML server (sends diagram content)" })).toBeVisible();
  expect(await page.evaluate(() => window.injected)).toBeUndefined();
  expect(external).toEqual([]);
});

test("diagram update targets only the selected diagram and preserves its ID", async ({ page }) => {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Two diagrams", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s", title: "Model", content: "", diagrams: [
          { id: "first", content: "@startuml\nclass Keep\n@enduml", diagram_type: "class", format: "plantuml" },
          { id: "second", content: "@startuml\nA -> B: Before\n@enduml", diagram_type: "sequence", format: "plantuml" },
        ] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.locator("textarea.prompt-input").fill("update the label");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect(page.locator(".error-bar")).toContainText("Select the diagram to update");
  await page.getByLabel("Diagram to update").selectOption("second");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.diagramType).toBe("sequence");
  expect(request.diagramId).toBe("second");
  expect(request.sectionId).toBe("s");
  await page.evaluate(() => window.resolveGeneration({ content: "@startuml\nA -> B: After\n@enduml", diagram_type: "sequence", format: "plantuml" }));
  await acceptChanges(page);
  await expect(page.getByRole("status")).toContainText("Updated Design");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  const diagrams = await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).documents[0].sections[0].diagrams);
  expect(diagrams[0].content).toContain("class Keep");
  expect(diagrams[1].id).toBe("second");
  expect(diagrams[1].content).toContain("After");
});

test("AI output stays in preview until accepted, with undo and redo", async ({ page }) => {
  await startGeneration(page);
  await finishGeneration(page);
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Generated content");
  await expect(dialog).toContainText("1 added");
  await dialog.getByRole("button", { name: "Accept changes", exact: true }).click();
  await expect(page.locator("h2.section-title")).toHaveText("Generated overview");
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect(page.locator("h2.section-title")).toHaveText("Generated overview");
});

test("editing after generation blocks stale acceptance; rejection preserves work", async ({ page }) => {
  await startGeneration(page);
  await finishGeneration(page);
  await page.getByRole("button", { name: "Slovensky", exact: true }).click();
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  await expect(page.getByRole("button", { name: "Accept changes", exact: true })).toBeDisabled();
  await expect(page.getByRole("dialog")).toContainText("Acceptance is blocked");
  await page.getByRole("dialog").getByRole("button", { name: "Discard result", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Start your architecture documentation" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Slovensky", exact: true })).toHaveClass(/active/);
});

test("manual section edits undo and redo without losing document identity", async ({ page }) => {
  await page.getByRole("button", { name: "Custom Your own template structure" }).click();
  await page.locator("h2.section-title").fill("Edited title");
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).documents[0].sections[0].title)).toBe("Overview");
  await expect(page.locator("h2.section-title")).toHaveText("Overview");
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect(page.locator("h2.section-title")).toHaveText("Edited title");
});

test("git history survives reload, restoration keeps newer versions and is undoable", async ({ page }) => {
  await page.getByLabel("Project name").fill("Original name");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  await page.getByLabel("Project name").fill("New name");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("New name");
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("New name");
  await page.getByRole("button", { name: "Git history", exact: true }).click();
  await page.getByRole("button", { name: /Save Original name$/ }).click();
  await page.getByRole("button", { name: "Restore this version", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("Original name");
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("New name");
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-project-history")).length)).toBe(3);
  const history = await page.evaluate(() => JSON.parse(localStorage.getItem("test-project-history")));
  expect(history.map(p => p.name)).toEqual(["Original name", "New name", "Original name"]);
  expect(history[2].revision).toBeGreaterThan(history[1].revision);
});

test("a save over content changed outside ArchGen is refused and keeps the edits", async ({ page }) => {
  await page.getByLabel("Project name").fill("Mine");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  await page.evaluate(() => {
    const pulled = JSON.parse(localStorage.getItem("test-saved-project"));
    localStorage.setItem("test-saved-project", JSON.stringify({ ...pulled, name: "Changed by git pull" }));
  });
  await page.getByLabel("Project name").fill("Mine, edited");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Save failed");
  await expect(page.getByLabel("Project name")).toHaveValue("Mine, edited");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("Changed by git pull");
});

test("a former .archgen file imports as an unsaved project that saves into a folder", async ({ page }) => {
  await page.evaluate(() => {
    window.confirmResult = true;
    window.loadFixture = { schemaVersion: 1, id: "legacy", name: "From SQLite", revision: 4, activeDocumentId: "d", documents: [
      { id: "d", name: "Old doc", revision: 2, template: "custom", language: "en", sections: [{ id: "s", title: "Kept", content: "Text", diagrams: [] }] }] };
  });
  await page.getByRole("button", { name: "Import .archgen", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Imported legacy.archgen");
  await expect(page.getByLabel("Project name")).toHaveValue("From SQLite");
  await expect(page.getByText("Unsaved changes")).toBeVisible();
  await expect(page.getByRole("button", { name: "Git history", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture-project");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("From SQLite");
});
