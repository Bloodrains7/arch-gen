import { test, expect } from "@playwright/test";

// UI integration tests mock IPC only. Real SQLite persistence is tested in Rust.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.isTauri = true;
    window.calls = [];
    window.confirmResult = true;
    const clone = value => JSON.parse(JSON.stringify(value));
    const sessions = new Map();
    const jobs = () => JSON.parse(localStorage.getItem("test-generation-jobs") ?? "[]");
    const putJob = job => {
      localStorage.setItem("test-generation-jobs", JSON.stringify([job, ...jobs().filter(j => j.id !== job.id)]));
      return clone(job);
    };
    for (const job of jobs()) if (["queued", "running"].includes(job.status)) putJob({ ...job, status: "interrupted" });
    const edit = (sessionId, expectedRevision, proposed, invalidateDocuments = false) => {
      const current = sessions.get(sessionId);
      if (current.revision !== expectedRevision) throw new Error("Revision conflict");
      const next = clone(proposed);
      next.revision = current.revision;
      for (const doc of next.documents) doc.revision = current.documents.find(d => d.id === doc.id)?.revision ?? 0;
      if (JSON.stringify(next) === JSON.stringify(current) && !invalidateDocuments) return clone(current);
      next.revision++;
      for (const doc of next.documents) {
        const old = current.documents.find(d => d.id === doc.id);
        if (invalidateDocuments) doc.revision = next.revision;
        else if (old && JSON.stringify(old) !== JSON.stringify(doc)) doc.revision++;
      }
      sessions.set(sessionId, clone(next));
      return next;
    };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" } },
      transformCallback: () => 1,
      invoke: async (command, args) => {
        window.calls.push({ command, args });
        if (command === "render_local_diagram") {
          if (window.renderFailure) throw new Error("Local renderer is not installed. No data was sent remotely.");
          if (window.deferRender) return new Promise(resolve => { window.resolveRender = resolve; });
          return '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="80"><script>window.injected=true</script><image href="https://example.invalid/leak"/><foreignObject><div>Unsafe</div></foreignObject><rect width="200" height="80" fill="url(https://example.invalid/fill)"/><text x="10" y="30">Local preview</text></svg>';
        }
        if (command.startsWith("plugin:event|")) return 1;
        if (command === "open_edit_session") {
          const sessionId = crypto.randomUUID();
          sessions.set(sessionId, clone(args.project));
          return { sessionId, project: clone(args.project) };
        }
        if (command === "apply_project_edit") {
          if (window.failNextEdit) { window.failNextEdit = false; throw new Error("Simulated journal failure"); }
          return edit(args.sessionId, args.expectedRevision, args.proposed, args.invalidateDocuments);
        }
        if (command === "list_generation_jobs") return jobs().filter(j => j.projectId === args.projectId);
        if (command === "create_generation_job") {
          const project = sessions.get(args.sessionId);
          const doc = project.documents.find(d => d.id === args.documentId);
          if (doc.revision !== args.expectedDocumentRevision) throw new Error("Document revision conflict");
          return putJob({ id: crypto.randomUUID(), projectId: project.id, documentId: doc.id,
            baseDocument: clone(doc), request: clone(args.request), status: "queued", sections: null,
            error: null, createdAt: "2026-09-05 12:00:00" });
        }
        if (command === "run_generation_job") {
          const job = jobs().find(j => j.id === args.jobId);
          putJob({ ...job, status: "running" });
          return new Promise(resolve => { window.resolveGeneration = raw => {
            const latest = jobs().find(j => j.id === args.jobId);
            if (latest.status !== "running") { resolve(latest); return; }
            let sections;
            if (job.request.kind === "documentation") {
              sections = raw.sections.map(s => ({ ...s, id: crypto.randomUUID(),
                diagrams: s.diagrams.map(g => ({ ...g, id: crypto.randomUUID() })) }));
            } else {
              sections = clone(job.baseDocument.sections);
              let section = sections.find(s => s.id === job.request.sectionId);
              if (!section) { section = { id: crypto.randomUUID(), title: "Diagram", content: "", diagrams: [] }; sections.push(section); }
              const existing = section.diagrams.findIndex(g => g.id === job.request.diagramId);
              const diagram = { ...raw, id: job.request.diagramId ?? crypto.randomUUID() };
              if (existing < 0) section.diagrams.push(diagram); else section.diagrams[existing] = diagram;
            }
            resolve(putJob({ ...latest, status: "ready", sections }));
          }; });
        }
        if (command === "cancel_generation_job" || command === "discard_generation_job") {
          return putJob({ ...jobs().find(j => j.id === args.jobId), status: command === "cancel_generation_job" ? "cancelled" : "discarded" });
        }
        if (command === "accept_generation_job" || command === "recover_generation_job") {
          const job = jobs().find(j => j.id === args.jobId);
          const project = clone(sessions.get(args.sessionId));
          if (job.status !== "ready" || job.projectId !== project.id) throw new Error("Unavailable result");
          if (command === "accept_generation_job") {
            const doc = project.documents.find(d => d.id === job.documentId);
            if (JSON.stringify(doc) !== JSON.stringify(job.baseDocument)) throw new Error("Stale result");
            doc.sections = clone(job.sections);
          } else {
            const doc = { ...clone(job.baseDocument), id: crypto.randomUUID(), name: `${job.baseDocument.name} (recovered)`, revision: 0,
              sections: job.sections.map(s => ({ ...clone(s), id: crypto.randomUUID(), diagrams: s.diagrams.map(g => ({ ...g, id: crypto.randomUUID() })) })) };
            project.documents.push(doc); project.activeDocumentId = doc.id;
          }
          const result = edit(args.sessionId, args.expectedRevision, project);
          putJob({ ...job, status: command === "accept_generation_job" ? "accepted" : "recovered" });
          return result;
        }
        if (command === "plugin:dialog|confirm") return window.confirmResult;
        if (command === "plugin:dialog|save" || command === "plugin:dialog|open") return "fixture.archgen";
        if (command === "save_project") {
          localStorage.setItem("test-saved-project", JSON.stringify(args.project));
          const history = JSON.parse(localStorage.getItem("test-project-history") ?? "[]");
          if (!history.some(p => p.id === args.project.id && p.revision === args.project.revision)) history.push(args.project);
          localStorage.setItem("test-project-history", JSON.stringify(history));
          return;
        }
        if (command === "list_project_history") {
          return JSON.parse(localStorage.getItem("test-project-history") ?? "[]").filter(p => p.id === args.projectId && (args.beforeRevision == null || p.revision < args.beforeRevision))
            .sort((a, b) => b.revision - a.revision).slice(0, 50).map(p => ({ revision: p.revision, savedAt: "2026-09-05 12:00:00" }));
        }
        if (command === "load_project_revision") {
          return JSON.parse(localStorage.getItem("test-project-history") ?? "[]").find(p => p.id === args.projectId && p.revision === args.revision);
        }
        if (command === "load_project") return window.loadFixture ?? JSON.parse(localStorage.getItem("test-saved-project"));
        if (command === "get_template") return ["Overview", "Data model"];
        throw new Error(`Unexpected IPC: ${command}`);
      },
    };
  });
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
  await expect(page.getByRole("status")).toContainText("Saved fixture.archgen");
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
  await expect(page.getByRole("status")).toContainText("Saved fixture.archgen");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("Preserved after failure");
});

test("save and reopen restores names, all documents and content", async ({ page }) => {
  await page.getByLabel("Project name").fill("Billing architecture");
  await page.getByRole("button", { name: "Custom Your own template structure" }).click();
  await page.getByTitle("New document").click();
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture.archgen");
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
  await expect(page.getByRole("status")).toContainText("Saved fixture.archgen");
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

test("saved history survives reload, restoration keeps newer versions and is undoable", async ({ page }) => {
  await page.getByLabel("Project name").fill("Original name");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("Saved fixture.archgen");
  const originalRevision = await page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).revision);
  await page.getByLabel("Project name").fill("New name");
  await page.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("test-saved-project")).name)).toBe("New name");
  await page.reload();
  await page.getByRole("button", { name: "Open project", exact: true }).click();
  await expect(page.getByLabel("Project name")).toHaveValue("New name");
  await page.getByRole("button", { name: "Saved history", exact: true }).click();
  await page.getByRole("button", { name: new RegExp(`^Revision ${originalRevision} ·`) }).click();
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
