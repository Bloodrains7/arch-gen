import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { installIpcMock } from "./ipc-mock.mjs";

const aiStatus = JSON.parse(readFileSync(new URL("../fixtures/ai-status.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  await page.addInitScript(installIpcMock, { aiStatus });
  await page.goto("/");
});

async function saveProject(page) {
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("Saved fixture-project");
}

test("applying a template adds its structure with guidance and keeps written content", async ({ page }) => {
  await page.evaluate(() => {
    window.loadFixture = { schemaVersion: 1, id: "p", name: "Billing", revision: 0, activeDocumentId: "d", documents: [
      { id: "d", name: "Design", revision: 0, template: "custom", language: "en", sections: [
        { id: "s1", title: "Overview", content: "Our billing system.", diagrams: [] },
        { id: "s2", title: "Scratch", content: "", diagrams: [] },
        { id: "s3", title: "System Context", content: "Users and the bank.", diagrams: [] },
      ] },
    ] };
  });
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await page.getByRole("button", { name: "C4 Model Context, Container, Component, Code" }).click();
  await expect(page.locator("h2.section-title")).toHaveText(["System Context", "Container Diagram", "Component Diagram", "Code / Class Diagram", "Overview"]);
  await expect(page.locator(".project-status").first()).toContainText("2 existing sections were kept");
  await saveProject(page);
  const sections = await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).documents[0].sections);
  expect(sections[0]).toMatchObject({ id: "s3", content: "Users and the bank." });
  expect(sections[1].content).toMatch(/^<!-- Deployable\/runnable units/);
  expect(sections[4]).toMatchObject({ id: "s1", content: "Our billing system." });
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator("h2.section-title")).toHaveText(["Overview", "Scratch", "System Context"]);
  // New architecture templates are available.
  for (const name of ["Solution Design", "Decision Record (ADR)", "Well-Architected Review", "Release Notes"]) {
    await expect(page.getByRole("button", { name: new RegExp(`^${name.replace(/[()]/g, "\\$&")} `) })).toBeVisible();
  }
});

test("release notes from a Git range become a new document without touching the repository", async ({ page }) => {
  await saveProject(page);
  await page.getByRole("button", { name: "Release notes…" }).click();
  const dialog = page.getByRole("dialog", { name: "Release notes from Git" });
  await expect(dialog.getByLabel("From revision")).toHaveValue("v1.1.0");
  await expect(dialog.getByLabel("To revision")).toHaveValue("HEAD");
  await dialog.getByLabel("Version").fill("1.2.0");
  await dialog.getByLabel("Release date").fill("2026-09-24");
  const preview = dialog.locator(".md-preview");
  await expect(preview.locator("h2")).toHaveText("[1.2.0] - 2026-09-24");
  await expect(preview.locator("h3")).toHaveText(["Added", "Fixed"]);
  await expect(preview).toContainText("api: add refunds (#12)");
  await expect(dialog.getByRole("status")).toHaveText("3 commits · 2 in the notes · 1 left out by the template (chore ×1)");
  expect(await page.evaluate(() => window.calls.find(c => c.command === "git_log_range").args))
    .toEqual({ path: "fixture-project", from: "v1.1.0", to: "HEAD", subpath: null, includeMerges: false });

  // Notes are never written for a range other than the one loaded.
  await dialog.getByLabel("From revision").fill("v1.0.0");
  await expect(dialog.getByRole("alert")).toContainText("Load commits again");
  await expect(dialog.getByRole("button", { name: "Create document" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Load commits" }).click();
  await expect(dialog.getByRole("button", { name: "Create document" })).toBeEnabled();
  expect(await page.evaluate(() => window.calls.filter(c => c.command === "git_log_range").at(-1).args.from)).toBe("v1.0.0");
  await dialog.getByRole("button", { name: "Create document" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab", { name: /Release notes 1\.2\.0/ })).toHaveClass(/active/);
  await expect(page.locator("h2.section-title")).toHaveText(["[1.2.0] - 2026-09-24"]);
  await saveProject(page);
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).documents[1]);
  expect(saved).toMatchObject({ name: "Release notes 1.2.0", template: "release-notes" });
  expect(saved.sections[0].content).toContain("### Fixed\n- rounding in totals");
  expect(await page.evaluate(() => window.calls.some(c => ["git_commit_project", "git_push_project"].includes(c.command)))).toBe(false);
});

test("an edited release template reports errors, can be shared through the project and inserted on top", async ({ page }) => {
  await page.getByRole("button", { name: "Custom Your own template structure" }).click();
  await saveProject(page);
  await page.getByRole("button", { name: "Release notes…" }).click();
  const dialog = page.getByRole("dialog", { name: "Release notes from Git" });
  await dialog.getByRole("button", { name: "Edit template" }).click();
  await dialog.getByLabel("Template source").fill("{{#groups}}broken");
  await expect(dialog.getByRole("alert")).toContainText("never closed");
  await expect(dialog.getByRole("button", { name: "Create document" })).toHaveCount(0);
  await dialog.getByLabel("Template source").fill("---\nname: Team\ngroups:\n  - title: Everything\n    types: \"*\"\n---\n# Team release {{version}}\n{{#commits}}\n- {{description}}\n{{/commits}}\n");
  await expect(dialog.locator(".md-preview h1")).toHaveText("Team release Unreleased");
  page.once("dialog", prompt => prompt.accept("Team notes"));
  await dialog.getByRole("button", { name: "Save template to project…" }).click();
  await expect(dialog.getByLabel("Release-note template")).toHaveValue("project:team-notes.md");
  expect(await page.evaluate(() => window.calls.find(c => c.command === "save_release_template").args))
    .toMatchObject({ path: "fixture-project", name: "Team notes", overwrite: false });
  await dialog.getByRole("button", { name: 'Insert at top of "Untitled"' }).click();
  await expect(page.locator("h2.section-title")).toHaveText(["Team release Unreleased", "Overview"]);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.locator("h2.section-title")).toHaveText(["Overview"]);
});

test("the Git bar commits only saved content and pushes only after confirmation", async ({ page }) => {
  await saveProject(page);
  await expect(page.locator(".git-status")).toHaveText("Git: main · 2 changes");
  await expect(page.getByRole("button", { name: "Push", exact: true })).toBeDisabled();

  await page.getByLabel("Project name").fill("Billing docs");
  await page.getByRole("button", { name: "Commit…" }).click();
  const dialog = page.getByRole("dialog", { name: "Commit project folder" });
  await expect(dialog.getByRole("alert")).toContainText("Save the project first");
  await expect(dialog.getByRole("button", { name: "Commit", exact: true })).toBeDisabled();
  await dialog.getByRole("button", { name: "Close" }).click();

  await saveProject(page);
  await page.getByRole("button", { name: "Commit…" }).click();
  await expect(dialog.getByRole("list", { name: "Changed files" })).toContainText("archgen.json");
  await expect(dialog.getByLabel("Commit message")).toHaveValue("docs: update Billing docs");
  await dialog.getByRole("button", { name: "Commit", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("Committed dddddddd (2 files): docs: update Billing docs");
  await expect(page.locator(".git-status")).toHaveText("Git: main · ↑1 · clean");

  await page.evaluate(() => { window.confirmResult = false; });
  await page.getByRole("button", { name: "Push", exact: true }).click();
  expect(await page.evaluate(() => window.gitPushed)).toBeUndefined();
  await page.evaluate(() => { window.confirmResult = true; });
  await page.getByRole("button", { name: "Push", exact: true }).click();
  await expect(page.locator(".project-status").first()).toContainText("Pushed main to origin.");
});

test("a folder outside Git shows why, and release notes explain it instead of failing silently", async ({ page }) => {
  await page.evaluate(() => { window.gitFixture = { statusError: "This folder is not in a Git repository. Run git init there, or choose another folder.", refsError: "This folder is not in a Git repository. Run git init there, or choose another folder." }; });
  await saveProject(page);
  await expect(page.locator(".git-status")).toHaveText("Git: This folder is not in a Git repository");
  await expect(page.getByRole("button", { name: "Commit…" })).toBeDisabled();
  await page.getByRole("button", { name: "Release notes…" }).click();
  const dialog = page.getByRole("dialog", { name: "Release notes from Git" });
  await expect(dialog.getByRole("alert")).toContainText("not in a Git repository");
  await expect(dialog.getByRole("button", { name: "Load commits" })).toBeDisabled();
});
