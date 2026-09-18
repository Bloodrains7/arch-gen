import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { installIpcMock } from "./ipc-mock.mjs";

const aiStatus = JSON.parse(readFileSync(new URL("../fixtures/ai-status.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  await page.addInitScript(installIpcMock, { aiStatus });
  await page.goto("/");
});

async function openThreeSectionFixture(page) {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Three sections", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s1", title: "Overview", content: "Overview text.", diagrams: [] },
        { id: "s2", title: "Data model", content: "Data model text.", diagrams: [] },
        { id: "s3", title: "Deployment", content: "Deployment text.", diagrams: [] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

async function openDiagramFixture(page) {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "With diagram", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s1", title: "Overview", content: "Overview text.", diagrams: [
          { id: "g1", content: "@startuml\nclass A\n@enduml", diagram_type: "class", format: "plantuml" },
        ] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

async function openTwoDocumentFixture(page) {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Two documents", revision: 0, activeDocumentId: "d1", documents: [
      { id: "d1", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s1", title: "Overview", content: "Overview text.", diagrams: [] },
      ] },
      { id: "d2", name: "Other", revision: 0, template: "custom", language: "en", sections: [
        { id: "s2", title: "Notes", content: "Notes text.", diagrams: [] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
}

test("ticking two of three sections sends only their ids and the preview shows exactly those two as changed", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.getByRole("checkbox", { name: "Select section 2: Data model for AI" }).check();
  await expect(page.getByRole("button", { name: "AI settings", exact: true })).toBeVisible();
  // Section 7: Auto/Diagram/Docs are disabled while something is ticked, the
  // provider chip is shown, and Generate is wired to the consent text.
  await expect(page.getByRole("button", { name: "Auto", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Diagram", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Docs", exact: true })).toBeDisabled();
  await expect(page.locator(".provider-chip")).toHaveText("Ollama (local) · qwen3.8:27b");
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toHaveAttribute("aria-describedby", "rework-consent");
  await page.locator("textarea.prompt-input").fill("Prepíš formálnejšie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");

  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request).toEqual({
    kind: "rework",
    instruction: "Prepíš formálnejšie.",
    language: "en",
    scope: "selection",
    sectionIds: ["s1", "s2"],
    diagramIds: [],
    includeContext: false,
    provider: "ollama",
    model: "qwen3.8:27b",
  });

  await page.evaluate(() => window.resolveGeneration({
    summary: "Rewrote two sections formally.",
    sections: [
      { id: "s1", title: "Overview", content: "Overview text, formally rewritten.", diagrams: [] },
      { id: "s2", title: "Data model", content: "Data model text, formally rewritten.", diagrams: [] },
      { id: "s3", title: "Deployment", content: "Deployment text.", diagrams: [] },
    ],
  }));
  await expect(page.locator(".pending-results")).toContainText("ollama (qwen3.8:27b)");
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("AI rework of Overview, Data model · ollama (qwen3.8:27b)");
  await expect(dialog).toContainText("Rewrote two sections formally.");
  await expect(dialog).toContainText("0 added");
  await expect(dialog).toContainText("2 changed");
  await expect(dialog).toContainText("1 unchanged");
  await expect(dialog.locator('[data-kind="changed"]')).toHaveCount(2);
  await expect(dialog.locator('[data-kind="unchanged"]')).toHaveCount(0);
  await dialog.getByRole("button", { name: "Accept changes", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Updated Design");
  await expect(page.locator("h2.section-title")).toHaveText(["Overview", "Data model", "Deployment"]);
});

test("consent text names the recipient for a cloud provider and stays empty for the local one", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await expect(page.locator("#rework-consent")).toHaveText("");

  await page.addInitScript(() => { window.aiStatusOverride = { provider: "anthropic" }; });
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await expect(page.locator("#rework-consent")).toContainText("Sends the selected content to Anthropic via Claude API.");
});

test("switching tabs drops the selection so Enter no longer sends a rework request", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.getByTitle("New document").click();
  await page.locator("textarea.prompt-input").press("Enter");
  await expect.poll(() => page.evaluate(() => window.calls.some(c => c.command === "create_generation_job"))).toBe(true);
  const reworkRequested = await page.evaluate(() => window.calls.some(c => c.command === "create_generation_job" && c.args.request.kind === "rework"));
  expect(reworkRequested).toBe(false);
});

test("selection is pruned when its section is removed", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.getByRole("checkbox", { name: "Select section 2: Data model for AI" }).check();
  await expect(page.locator(".selection-summary")).toHaveText("2 sections");
  await page.locator(".section-card").first().getByTitle("Remove section").click();
  await expect(page.locator(".selection-summary")).toHaveText("1 section");
  await expect(page.getByRole("checkbox", { name: "Select section 1: Data model for AI" })).toBeChecked();
});

test("pressing Enter while the configured provider is unavailable creates no job", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.addInitScript(() => { window.aiStatusOverride = { provider: "openai" }; });
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await expect(page.locator(".rework-blocked")).toContainText("Set an OpenAI API key");
  await page.locator("textarea.prompt-input").press("Enter");
  await expect(page.locator(".rework-blocked")).toBeVisible();
  expect(await page.evaluate(() => window.calls.some(c => c.command === "create_generation_job"))).toBe(false);
});

test("settings modal switches provider and never renders a key", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await expect(dialog.getByRole("radio", { name: "Ollama (local)", exact: true })).toBeChecked();

  await dialog.getByRole("radio", { name: "Claude API", exact: true }).check();
  await expect(dialog.getByRole("radio", { name: "Claude API", exact: true })).toBeChecked();

  const secretKey = "sk-test-super-secret-value";
  await dialog.getByLabel("API key", { exact: true }).fill(secretKey);
  await dialog.getByRole("button", { name: "Save key", exact: true }).click();
  await expect(dialog.getByLabel("API key", { exact: true })).toHaveValue("");
  expect(await page.locator("body").innerText()).not.toContain(secretKey);
  expect(await page.content()).not.toContain(secretKey);

  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.locator("#rework-consent")).toContainText("Sends the whole document to Anthropic via Claude API.");

  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  await dialog.getByRole("radio", { name: "Ollama (local)", exact: true }).check();
  await expect(dialog.getByRole("radio", { name: "Ollama (local)", exact: true })).toBeChecked();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.locator("#rework-consent")).toHaveText("");

  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  await dialog.getByRole("button", { name: "Test provider", exact: true }).click();
  await expect(dialog.getByText("Test succeeded using qwen3.8:27b.")).toBeVisible();

  await page.evaluate(() => { window.aiTestFailure = "Ollama is not running at http://localhost:11434."; });
  await dialog.getByRole("button", { name: "Test provider", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("Ollama is not running at http://localhost:11434.");
});

test("ticking a section then removing it before pressing Enter creates no rework request", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis dokumentaciu.");
  await page.locator(".section-card").first().getByTitle("Remove section").click();
  await page.locator("textarea.prompt-input").press("Enter");
  await expect.poll(() => page.evaluate(() => window.calls.some(c => c.command === "create_generation_job"))).toBe(true);
  const reworkRequested = await page.evaluate(() => window.calls.some(c => c.command === "create_generation_job" && c.args.request.kind === "rework"));
  expect(reworkRequested).toBe(false);
});

test("mode left on Rework after an earlier whole-document request still scopes a later selection to only the ticked blocks", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await expect(page.locator(".selection-summary")).toHaveText("Whole document");
  await page.getByRole("checkbox", { name: "Select section 2: Data model for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");

  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.scope).toBe("selection");
  expect(request.sectionIds).toEqual(["s2"]);
  expect(request.diagramIds).toEqual([]);
});

test("checkbox accessible names stay unique when two sections share a title", async ({ page }) => {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Duplicate titles", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s1", title: "Overview", content: "First.", diagrams: [] },
        { id: "s2", title: "Overview", content: "Second.", diagrams: [] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI", exact: true }).check();
  await page.getByRole("checkbox", { name: "Select section 2: Overview for AI", exact: true }).check();
  await expect(page.locator(".selection-summary")).toHaveText("2 sections");
  await expect(page.getByRole("checkbox", { name: "Select section 1: Overview for AI", exact: true })).toBeChecked();
  await expect(page.getByRole("checkbox", { name: "Select section 2: Overview for AI", exact: true })).toBeChecked();
});

test("rework is blocked when AI status fails to load", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.addInitScript(() => { window.aiStatusFailure = "The AI service could not be reached."; });
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await expect(page.locator(".rework-blocked")).toContainText("Could not load AI status");
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeDisabled();
  await page.locator("textarea.prompt-input").press("Enter");
  expect(await page.evaluate(() => window.calls.some(c => c.command === "create_generation_job"))).toBe(false);
});

test("opening AI settings retries a previously failed AI status load", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.addInitScript(() => { window.aiStatusFailure = "The AI service could not be reached."; });
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await expect(page.locator(".rework-blocked")).toContainText("Could not load AI status");

  // The file behind the failure is now fixed; only opening the dialog (design
  // doc §9) can recover from the dead end, since the rejected-creation refresh
  // never fires while every rework request is blocked.
  await page.evaluate(() => { window.aiStatusFailure = undefined; });
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "AI settings" })).toBeVisible();
  await expect(page.locator(".rework-blocked")).toHaveCount(0);
});

test("AI settings falls back to manual model entry and shows the error when ai_ollama_models fails", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.evaluate(() => { window.ollamaModelsFailure = "Ollama is not running at http://localhost:11434."; });
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await expect(dialog.locator("select")).toHaveCount(0);
  await expect(dialog.getByPlaceholder("default model")).toBeVisible();
  await expect(dialog.getByRole("alert")).toContainText("Ollama is not running at http://localhost:11434.");
});

test("whole-document rework via the Rework button with nothing ticked sends an empty scope", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.locator("textarea.prompt-input").fill("Prepis cely dokument formalnejsie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");

  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.kind).toBe("rework");
  expect(request.scope).toBe("document");
  expect(request.sectionIds).toEqual([]);
  expect(request.diagramIds).toEqual([]);
});

// The three variants below all exercise the same critical defect: once Rework
// has been clicked, an emptied selection must never silently fall back to
// "whole document" (design doc §9, "Scope and consent"). Whole-document scope
// is armed only by clicking Rework with nothing ticked, and that arm is
// dropped by the very next toggle — so ticking a block *after* Rework leaves
// nothing armed once the tick is undone by any of these three events.
async function armRework(page) {
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
}

async function assertReworkBlockedAndNoRequest(page) {
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeDisabled();
  await expect(page.locator(".rework-blocked")).toContainText("Tick blocks, or click Rework to rework the whole document.");
  // Counts calls rather than searching for a rework job, so this also catches
  // an earlier-in-the-test job (the diagram test below creates one on
  // purpose) — Enter must add no IPC call at all while blocked.
  const callsBefore = await page.evaluate(() => window.calls.length);
  await page.locator("textarea.prompt-input").press("Enter");
  const callsAfter = await page.evaluate(() => window.calls.length);
  expect(callsAfter).toBe(callsBefore);
}

test("Rework clicked, then ticked, then a tab switch: the selection loss does not widen to the whole document", async ({ page }) => {
  await openThreeSectionFixture(page);
  await armRework(page);
  await page.getByTitle("New document").click();
  await assertReworkBlockedAndNoRequest(page);
});

test("Rework clicked, then ticked, then the section removed: the selection loss does not widen to the whole document", async ({ page }) => {
  await openThreeSectionFixture(page);
  await armRework(page);
  await page.locator(".section-card").first().getByTitle("Remove section").click();
  await assertReworkBlockedAndNoRequest(page);
});

test("Rework clicked, then ticked, then unticked: the selection loss does not widen to the whole document", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  const checkbox = page.getByRole("checkbox", { name: "Select section 1: Overview for AI" });
  await checkbox.check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await checkbox.uncheck();
  await assertReworkBlockedAndNoRequest(page);
});

test("ticking a diagram and the context checkbox sends diagramIds and includeContext, and a created request spends both the context tick and the whole-document arm", async ({ page }) => {
  await openDiagramFixture(page);
  // Click Rework first (as the critical-defect tests above do) so `userMode`
  // stays "rework" after the diagram is unticked below, instead of falling
  // back to Auto and silently trying a Docs/Diagram job. Switch the *real*
  // configured provider through the settings dialog (not the display-only
  // `aiStatusOverride`), so the mock's own provider check does not reject the
  // created job as "AI settings changed after this request was created."
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const settingsDialog = page.getByRole("dialog", { name: "AI settings" });
  await settingsDialog.getByRole("radio", { name: "Claude API", exact: true }).check();
  await settingsDialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select diagram 1 (class) in section 1: Overview for AI" }).check();
  await page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" }).check();
  await expect(page.locator("#rework-consent")).toContainText("the selected content and the rest of the document (as context)");
  await page.locator("textarea.prompt-input").fill("Add a field.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");

  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request).toEqual({
    kind: "rework",
    instruction: "Add a field.",
    language: "en",
    scope: "selection",
    sectionIds: [],
    diagramIds: ["g1"],
    includeContext: true,
    provider: "anthropic",
    model: "claude-opus-5",
  });

  // Let the job finish so Generate returns to its idle, re-clickable state;
  // otherwise the assertions below would trivially pass because a request is
  // already in flight, rather than because scope "none" blocks a new one.
  await page.evaluate(() => window.resolveGeneration({
    summary: "Added a field.",
    sections: [{ id: "s1", title: "Overview", content: "Overview text.", diagrams: [
      { id: "g1", content: "@startuml\nclass A\nclass B\n@enduml", diagram_type: "class", format: "plantuml" },
    ] }],
  }));
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeEnabled();

  // The created request spent the context tick; unticking the diagram again
  // must not find the whole-document arm still standing from before.
  await page.getByRole("checkbox", { name: "Select diagram 1 (class) in section 1: Overview for AI" }).uncheck();
  await expect(page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" })).toHaveCount(0);
  await assertReworkBlockedAndNoRequest(page);
});

test("ticking a section covers its diagram: the diagram checkbox becomes checked and disabled, and only the section id is sent", async ({ page }) => {
  await openDiagramFixture(page);
  // Section 7: the "Diagram to update" select (the Docs/Diagram-mode control)
  // is visible before any tick and hidden once rework mode takes over.
  await expect(page.getByLabel("Diagram to update")).toBeVisible();
  const diagramCheckbox = page.getByRole("checkbox", { name: "Select diagram 1 (class) in section 1: Overview for AI" });
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await expect(page.getByLabel("Diagram to update")).toHaveCount(0);
  await expect(diagramCheckbox).toBeChecked();
  await expect(diagramCheckbox).toBeDisabled();
  await expect(page.locator(".selection-summary")).toHaveText("1 section");
  await page.locator("textarea.prompt-input").fill("Rewrite it.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.sectionIds).toEqual(["s1"]);
  expect(request.diagramIds).toEqual([]);
});

test("the context checkbox does not carry over from one document to another", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" }).check();
  await page.getByRole("tab").nth(1).click();
  await page.getByRole("checkbox", { name: "Select section 1: Notes for AI" }).check();
  await expect(page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" })).not.toBeChecked();
});

test("the local-engine hint is shown in Auto mode and hidden once Rework is selected", async ({ page }) => {
  await openThreeSectionFixture(page);
  await expect(page.getByText("Local Ollama (Python engine)", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await expect(page.getByText("Local Ollama (Python engine)", { exact: true })).toHaveCount(0);
});

test("job summary line breaks are preserved (a removal note must not run into the model's prose)", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  await page.evaluate(() => window.resolveGeneration({
    summary: "Reworded the text.\nRemoves 1 section(s): Data model",
    sections: [
      { id: "s1", title: "Overview", content: "Overview text, rewritten.", diagrams: [] },
      { id: "s3", title: "Deployment", content: "Deployment text.", diagrams: [] },
    ],
  }));
  await expect(page.locator(".pending-results .project-status").first()).toHaveCSS("white-space", "pre-line");
  await page.getByRole("button", { name: "Review changes", exact: true }).click();
  await expect(page.locator(".job-summary")).toHaveCSS("white-space", "pre-line");
});

test("leaving a project while a rework request is still running does not leave the new project's Generate stuck disabled", async ({ page }) => {
  await openThreeSectionFixture(page);
  // Tick without clicking the Rework button, so `userMode` stays "auto": once
  // the new project drops the selection, only a stuck `isGenerating` (not the
  // scope-"none" hint) could still be disabling Generate.
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  await expect(page.locator("button.generate-btn")).toBeDisabled();

  // The old job is left running (never resolved) — its provider request may
  // still be in flight against a paid API.
  await page.getByRole("button", { name: "New project", exact: true }).click();
  await expect(page.getByRole("button", { name: "New project", exact: true })).toBeEnabled();
  await expect(page.locator("button.generate-btn")).toBeEnabled();
});

// settings-dialog-and-e2e-quality#2: a rejected provider switch must not leave
// the browser's own radio state lying about which provider is really
// configured — Svelte's one-way `checked={...}` skips the DOM write once the
// browser has already flipped the radio itself, so the dialog has to correct
// it by hand in the catch.
test("a failed provider switch leaves the dialog on the provider that is really configured, and a retry succeeds", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await expect(dialog.getByRole("radio", { name: "Ollama (local)", exact: true })).toBeChecked();

  await page.evaluate(() => { window.aiConfigureFailure = "writing AI settings: Access is denied. (os error 5)"; });
  await dialog.getByRole("radio", { name: "Claude API", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("Access is denied");
  await expect(dialog.getByRole("radio", { name: "Ollama (local)", exact: true })).toBeChecked();
  await expect(dialog.getByRole("radio", { name: "Claude API", exact: true })).not.toBeChecked();
  expect(await page.evaluate(() => window.calls.filter(c => c.command === "ai_configure").length)).toBe(1);

  await page.evaluate(() => { window.aiConfigureFailure = undefined; });
  await dialog.getByRole("radio", { name: "Claude API", exact: true }).click();
  await expect(dialog.getByRole("radio", { name: "Claude API", exact: true })).toBeChecked();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
});

// settings-dialog-and-e2e-quality#3: the field must never expose a typed or
// saved key, and every save/remove must be recorded under the exact provider
// the dialog was showing — including a key the mock's own character check
// rejects, and a key that failed to save but must still vanish on reopen.
test("the API key field never leaks its value, and every save targets exactly the provider the dialog is showing", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await dialog.getByRole("radio", { name: "OpenAI API", exact: true }).click();
  await expect(dialog.getByRole("radio", { name: "OpenAI API", exact: true })).toBeChecked();

  const keyField = dialog.getByLabel("API key", { exact: true });
  await expect(keyField).toHaveAttribute("type", "password");
  await expect(dialog.getByText("No key is set.")).toBeVisible();

  const secretKey = "sk-test-super-secret-value";
  const hasValueAnywhere = () => page.evaluate(k => [...document.querySelectorAll("input,textarea")].some(el => el.value === k), secretKey);
  await keyField.fill(secretKey);
  expect(await hasValueAnywhere()).toBe(true); // positive control: the scan can see a value that really is there

  await dialog.getByRole("button", { name: "Save key", exact: true }).click();
  await expect(keyField).toHaveValue("");
  await expect(dialog.getByText("Using a key stored in AI settings.")).toBeVisible();
  expect(await hasValueAnywhere()).toBe(false);
  expect(await page.locator("body").innerText()).not.toContain(secretKey);
  expect(await page.content()).not.toContain(secretKey);

  const setKeyCalls = await page.evaluate(() => window.calls.filter(c => c.command === "ai_set_key").map(c => c.args));
  expect(setKeyCalls).toEqual([{ provider: "openai", key: secretKey }]);

  await dialog.getByRole("button", { name: "Remove key", exact: true }).click();
  await expect(dialog.getByText("No key is set.")).toBeVisible();

  // A key the backend would reject (a space) must fail visibly and never be echoed.
  const badKey = "sk-has a space";
  await keyField.fill(badKey);
  await dialog.getByRole("button", { name: "Save key", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("printable ASCII text with no spaces");
  expect(await page.locator("body").innerText()).not.toContain(badKey);

  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  await expect(keyField).toHaveValue("");
});

// contract-completeness#6 / settings-dialog-and-e2e-quality#10: a failed
// `ai_status` must never be a dead end. Opening the dialog already retries
// (design doc §9); this covers the case where that retry itself still fails,
// so the dialog has to show the reason and offer its own "Try again".
test("AI settings shows the load error and offers a retry when opening the dialog does not fix it", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.addInitScript(() => { window.aiStatusFailure = "The AI settings file is damaged and was left untouched: unexpected end of file. Fix or delete ai-settings.json."; });
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await expect(page.locator(".rework-blocked")).toContainText("damaged");

  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await expect(dialog.getByRole("alert")).toContainText("damaged");
  await expect(dialog.getByRole("radiogroup")).toHaveCount(0);

  // The file is fixed without restarting ArchGen; "Try again" must recover
  // without the user having to close and reopen the dialog.
  await page.evaluate(() => { window.aiStatusFailure = undefined; });
  await dialog.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(dialog.getByRole("radiogroup")).toBeVisible();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.locator(".rework-blocked")).toHaveCount(0);
});

// settings-dialog-and-e2e-quality#5: the mock's provider-mismatch branch (and
// App's re-fetch of a rejected rework creation) had no coverage at all — every
// rework request in the suite ran under the default, never-switched provider.
test("a rework request rejected because AI settings changed behind the UI refreshes the status and the provider chip", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await expect(page.locator(".provider-chip")).toHaveText("Ollama (local) · qwen3.8:27b");

  // Change the backend's real configured provider directly (not the
  // display-only `aiStatusOverride`), so App's own `aiStatus` is now stale.
  await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("ai_configure", { provider: "anthropic" }));
  const statusCallsBefore = await page.evaluate(() => window.calls.filter(c => c.command === "ai_status").length);

  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect(page.locator(".error-bar")).toContainText("AI settings changed after this request was created. Create the request again.");

  const statusCallsAfter = await page.evaluate(() => window.calls.filter(c => c.command === "ai_status").length);
  expect(statusCallsAfter).toBeGreaterThan(statusCallsBefore);
  await expect(page.locator(".provider-chip")).toHaveText("Claude API · claude-opus-5");
  await expect(page.locator("#rework-consent")).toContainText("Sends the selected content to Anthropic via Claude API.");
  expect(JSON.parse(await page.evaluate(() => localStorage.getItem("test-generation-jobs") ?? "[]")).length).toBe(0);
});

// settings-dialog-and-e2e-quality#6: the mock used to accept a provider id,
// model or Ollama URL the backend would reject, and had no way to reject a
// key or a provider the UI itself can never send — every one of those error
// branches was unreachable from a test.
test("AI settings surfaces the backend's own rejection under the right field, for a bad model, URL, provider and key", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });

  // Model: a space is rejected the same way `valid_model` rejects it.
  await dialog.getByRole("radio", { name: "Codex CLI", exact: true }).click();
  const modelField = dialog.getByPlaceholder("default model");
  await modelField.fill("opus 5");
  await modelField.blur();
  await expect(dialog.getByRole("alert")).toContainText('"opus 5" is not a valid model name.');

  // Ollama URL: a non-loopback host is rejected, never repaired.
  await dialog.getByRole("radio", { name: "Ollama (local)", exact: true }).click();
  const urlField = dialog.getByLabel("Ollama URL", { exact: true });
  await urlField.fill("http://evil.example");
  await urlField.blur();
  await expect(dialog.getByRole("alert")).toContainText("Ollama is local: the URL must be http://localhost");

  // A provider id the radios could never send, and a key for a provider that
  // does not use one: the UI can never construct either call itself, so these
  // are exercised directly to prove the mock (like the backend) still refuses them.
  const providerRejected = await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("ai_configure", { provider: "bogus" }).then(() => "resolved", e => String(e)));
  expect(providerRejected).toContain('"bogus" is not a known AI provider.');
  const keyRejected = await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("ai_set_key", { provider: "ollama", key: "x" }).then(() => "resolved", e => String(e)));
  expect(keyRejected).toContain("does not use an API key.");
});

// settings-dialog-and-e2e-quality#7: the field must clamp client-side so the
// backend's raw `Option<u64>` rejection can never reach the user, and the
// clamped value it actually stored must always be the one shown.
test("the timeout field never sends a float, NaN or out-of-range value, and always shows the value actually configured", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  const timeoutField = dialog.getByLabel("Timeout (seconds)", { exact: true });
  await expect(timeoutField).toHaveValue("600");

  await timeoutField.fill("-1");
  await timeoutField.blur();
  await expect(timeoutField).toHaveValue("30");

  await timeoutField.fill("10");
  await timeoutField.blur();
  await expect(timeoutField).toHaveValue("30"); // clamps to the same value again — must still show it, not "10"

  await timeoutField.fill("99999999999999999999");
  await timeoutField.blur();
  await expect(timeoutField).toHaveValue("3600");

  await timeoutField.fill("");
  await timeoutField.blur();
  await expect(dialog.getByRole("alert")).toContainText("Enter a number of seconds between 30 and 3600.");
  await expect(timeoutField).toHaveValue("3600"); // reverted to what is really configured, not left blank

  const sentTimeouts = await page.evaluate(() => window.calls.filter(c => c.command === "ai_configure").map(c => c.args.timeoutSeconds).filter(v => v !== undefined));
  expect(sentTimeouts.every(v => Number.isInteger(v) && v >= 30 && v <= 3600)).toBe(true);
  expect(sentTimeouts).toEqual([30, 30, 3600]);
});

// settings-dialog-and-e2e-quality#8: a rejected model and its alert used to
// survive a provider switch, and a late "Test provider" reply could land
// under whichever provider happened to be showing when it arrived.
test("switching providers clears the previous provider's field errors and shows a fresh Model field, not the old rejected value", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });

  await dialog.getByRole("radio", { name: "Claude CLI", exact: true }).click();
  const modelField = dialog.getByPlaceholder("default model");
  await modelField.fill("opus 5");
  await modelField.blur();
  await expect(dialog.getByRole("alert")).toContainText('"opus 5" is not a valid model name.');

  await dialog.getByRole("radio", { name: "Codex CLI", exact: true }).click();
  await expect(dialog.getByRole("radio", { name: "Codex CLI", exact: true })).toBeChecked();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(dialog.getByPlaceholder("default model")).toHaveValue("");
  expect(await page.evaluate(() => window.calls.filter(c => c.command === "ai_configure" && c.args.model !== undefined).length)).toBe(1);

  // A Test started on Codex CLI must not appear once Gemini CLI is showing.
  await page.evaluate(() => { window.deferTest = true; });
  await dialog.getByRole("button", { name: "Test provider", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveTest)).toBe("function");
  await dialog.getByRole("radio", { name: "Gemini CLI", exact: true }).click();
  await expect(dialog.getByRole("radio", { name: "Gemini CLI", exact: true })).toBeChecked();
  await page.evaluate(() => window.resolveTest());
  await expect(dialog.locator('[role="status"]', { hasText: "Test succeeded" })).toHaveCount(0);
});

// settings-dialog-and-e2e-quality#9: the model list used to load once per
// dialog lifetime, with no way to retry a failure or reload after the URL
// changed, and a configured model missing from a fresh list blanked the select.
test("the Ollama model list can be retried after a failure and reloads once the URL is fixed", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.evaluate(() => { window.ollamaModelsFailure = "Ollama is not running at http://localhost:11434."; });
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await expect(dialog.locator("select")).toHaveCount(0);
  await expect(dialog.getByRole("alert")).toContainText("Ollama is not running");

  // Fixing the URL alone must retry, without a manual reload.
  await page.evaluate(() => { window.ollamaModelsFailure = undefined; });
  const urlField = dialog.getByLabel("Ollama URL", { exact: true });
  await urlField.fill("http://127.0.0.1:11434");
  await urlField.blur();
  await expect(dialog.locator("select")).toBeVisible();
  await expect(dialog.locator("select")).toHaveValue("qwen3.8:27b");
  expect(await page.evaluate(() => window.calls.filter(c => c.command === "ai_ollama_models").length)).toBe(2);

  // A model the dialog knows is configured but that Ollama no longer reports
  // must not blank the select: it stays selected, marked as not installed.
  await page.evaluate(() => { window.ollamaModelsOverride = ["other:1b"]; });
  await dialog.getByRole("button", { name: "Reload models", exact: true }).click();
  await expect(dialog.locator("select")).toHaveValue("qwen3.8:27b");
  await expect(dialog.locator("select option", { hasText: "not installed" })).toHaveCount(1);
});

// settings-dialog-and-e2e-quality#11: `aria-label` on the radio hid the
// availability/detail text from assistive tech, and closing the dialog
// dropped focus to <body> instead of returning it to the opener.
test("provider radios describe availability and detail to assistive tech, and focus returns to the AI settings button on close", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  const openButton = page.getByRole("button", { name: "AI settings", exact: true });
  await openButton.click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  const openaiRadio = dialog.getByRole("radio", { name: "OpenAI API", exact: true });
  const describedBy = await openaiRadio.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  const description = await page.evaluate(ids => ids.split(" ").map(id => document.getElementById(id)?.textContent).join(" "), describedBy);
  expect(description).toContain("Not available");
  expect(description).toContain("Set an OpenAI API key");

  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(openButton).toBeFocused();
});

// Extra adversarial coverage beyond the review findings: `includeContext` is
// tagged only by session+document (design doc §9), deliberately not by
// provider, so ticking it under a local provider and then switching to a
// cloud one in AI settings must still be fully visible — the checkbox stays
// checked, the consent line names the new recipient and mentions the context,
// and the captured request agrees with both.
test("the context checkbox and consent text follow a provider switch within the same document, never silently", async ({ page }) => {
  await openThreeSectionFixture(page);
  await page.getByRole("checkbox", { name: "Select section 1: Overview for AI" }).check();
  await page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" }).check();
  await expect(page.locator("#rework-consent")).toHaveText(""); // Ollama is local: nothing to consent to yet

  await page.getByRole("button", { name: "AI settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "AI settings" });
  await dialog.getByRole("radio", { name: "Claude API", exact: true }).check();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();

  // The tick must still be visible, and the consent line must name both the
  // new recipient and the context before anything is sent.
  const contextBox = page.getByRole("checkbox", { name: "Include the rest of the document as read-only context" });
  await expect(contextBox).toBeChecked();
  await expect(page.locator("#rework-consent")).toHaveText(
    "Sends the selected content and the rest of the document (as context) to Anthropic via Claude API.",
  );

  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.provider).toBe("anthropic");
  expect(request.sectionIds).toEqual(["s1"]);
  expect(request.includeContext).toBe(true);
});

// Whole-document rework is armed per session+document (design doc §9): moving
// to a different tab must never let it apply there, and moving back must
// resume exactly the same, visible choice rather than a silently different one.
test("a whole-document rework armed on one tab is blocked on another and resumes correctly on returning to the first", async ({ page }) => {
  await openTwoDocumentFixture(page);
  await page.getByRole("button", { name: "Rework", exact: true }).click();
  await expect(page.locator(".selection-summary")).toHaveText("Whole document");

  await page.getByRole("tab").nth(1).click();
  await expect(page.locator(".rework-blocked")).toContainText("Tick blocks, or click Rework to rework the whole document.");
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeDisabled();
  await page.locator("textarea.prompt-input").fill("Prepis formalnejsie.");
  await page.locator("textarea.prompt-input").press("Enter");
  expect(await page.evaluate(() => window.calls.some(c => c.command === "create_generation_job"))).toBe(false);

  await page.getByRole("tab").nth(0).click();
  await expect(page.locator(".selection-summary")).toHaveText("Whole document");
  await expect(page.getByRole("button", { name: "Generate", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Generate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof window.resolveGeneration)).toBe("function");
  const request = await page.evaluate(() => window.calls.find(c => c.command === "create_generation_job").args.request);
  expect(request.scope).toBe("document");
  expect(request.sectionIds).toEqual([]);
  expect(request.diagramIds).toEqual([]);
});
