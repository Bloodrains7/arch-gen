// The IPC mock shared by projects.spec.mjs and rework.spec.mjs. It runs inside
// the browser via `page.addInitScript(installIpcMock, fixtures)`: Playwright
// serialises the function itself, so it must be self-contained and cannot
// close over anything from this module's scope — fixture data (read here with
// node:fs by the spec files) travels through the `fixtures` argument instead.
export function installIpcMock(fixtures) {
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
  // Mutable AI configuration state for this page. `ai_status` always reflects
  // it, so `ai_configure`/`ai_set_key` calls the AiSettings dialog makes are
  // visible on the next status fetch, the same way the real backend works.
  let aiStatus = clone(fixtures.aiStatus);
  // A per-test escape hatch (in the spirit of `window.loadFixture` and
  // `window.confirmResult` below) for tests that need a non-default current
  // provider without driving the AI settings UI: shallow-merged onto the
  // status every `ai_status` call returns.
  const currentAiStatus = () => clone({ ...aiStatus, ...(window.aiStatusOverride || {}) });
  const noUnknownArgs = (args, allowed) => {
    for (const key of Object.keys(args ?? {})) {
      if (!allowed.includes(key)) throw new Error(`Unexpected argument: ${key}`);
    }
  };
  // Mirrors of the validation `ai_configure`/`ai_set_key`/`create_job_as` do on
  // the Rust side (settings.rs, runtime.rs), kept just accurate enough that
  // every error branch AiSettings and the rework bar can hit is reachable from
  // a test, without trying to be a second copy of the full backend.
  const REJECTED_OLLAMA_URL = "Ollama is local: the URL must be http://localhost, http://127.0.0.1 or http://[::1], with an optional :port and nothing after it but a single trailing /.";
  const validModel = model => model.length > 0 && model.length <= 120 && !model.startsWith("-") && /^[A-Za-z0-9._:/-]+$/.test(model);
  const validKey = key => key.length >= 1 && key.length <= 512 && [...key].every(c => c.charCodeAt(0) >= 0x21 && c.charCodeAt(0) <= 0x7e);
  const localOllamaUrl = url => {
    if (/[\x00-\x1f\x7f]/.test(url)) throw new Error(REJECTED_OLLAMA_URL);
    const schemeSplit = url.indexOf("://");
    if (schemeSplit === -1) throw new Error(REJECTED_OLLAMA_URL);
    const scheme = url.slice(0, schemeSplit);
    const rest = url.slice(schemeSplit + 3);
    if (scheme !== "http" && scheme !== "https") throw new Error(REJECTED_OLLAMA_URL);
    const splitIndex = (() => { const m = rest.search(/[/?#]/); return m === -1 ? rest.length : m; })();
    const authority = rest.slice(0, splitIndex);
    const tail = rest.slice(splitIndex);
    if (!(tail === "" || tail === "/") || authority.includes("@")) throw new Error(REJECTED_OLLAMA_URL);
    let host, port;
    if (authority.startsWith("[")) {
      const inside = authority.slice(1);
      const close = inside.indexOf("]");
      if (close === -1) throw new Error(REJECTED_OLLAMA_URL);
      host = `[${inside.slice(0, close)}]`; port = inside.slice(close + 1);
    } else {
      const colon = authority.indexOf(":");
      if (colon === -1) { host = authority; port = ""; } else { host = authority.slice(0, colon); port = authority.slice(colon); }
    }
    if (host !== "localhost" && host !== "127.0.0.1" && host !== "[::1]") throw new Error(REJECTED_OLLAMA_URL);
    if (port !== "") {
      const digits = port.slice(1);
      if (!(digits.length > 0 && /^[0-9]+$/.test(digits) && Number(digits) >= 1 && Number(digits) <= 65535)) throw new Error(REJECTED_OLLAMA_URL);
    }
    return `${scheme}://${authority}`;
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
      if (command === "ai_status") {
        noUnknownArgs(args, []);
        // Same escape-hatch pattern as `window.aiTestFailure`: lets a test drive
        // App's "AI status failed to load" path without a real backend failure.
        if (window.aiStatusFailure) throw new Error(window.aiStatusFailure);
        return currentAiStatus();
      }
      if (command === "ai_configure") {
        noUnknownArgs(args, ["provider", "model", "ollamaUrl", "timeoutSeconds"]);
        if (window.aiConfigureFailure) throw new Error(window.aiConfigureFailure);
        // Validate everything before applying anything, like `apply_configure`.
        let targetProvider = aiStatus.provider;
        if (args.provider !== undefined) {
          if (!aiStatus.providers.some(p => p.id === args.provider)) throw new Error(`"${args.provider}" is not a known AI provider.`);
          targetProvider = args.provider;
        }
        let trimmedModel;
        if (args.model !== undefined) {
          trimmedModel = args.model.trim();
          if (trimmedModel !== "" && !validModel(trimmedModel)) throw new Error(`"${trimmedModel}" is not a valid model name.`);
        }
        let resolvedUrl;
        if (args.ollamaUrl !== undefined) {
          resolvedUrl = args.ollamaUrl.trim() === "" ? "http://localhost:11434" : localOllamaUrl(args.ollamaUrl);
        }
        if (args.timeoutSeconds !== undefined && (!Number.isInteger(args.timeoutSeconds) || args.timeoutSeconds < 0)) {
          throw new Error(`invalid args \`timeoutSeconds\` for command \`ai_configure\`: invalid value: expected u64`);
        }
        aiStatus.provider = targetProvider;
        if (args.model !== undefined) {
          const provider = aiStatus.providers.find(p => p.id === targetProvider);
          if (provider) provider.model = trimmedModel || provider.defaultModel;
        }
        if (resolvedUrl !== undefined) aiStatus.ollamaUrl = resolvedUrl;
        if (args.timeoutSeconds !== undefined) aiStatus.timeoutSeconds = Math.min(3600, Math.max(30, args.timeoutSeconds));
        return currentAiStatus();
      }
      if (command === "ai_set_key") {
        noUnknownArgs(args, ["provider", "key"]);
        if (window.aiSetKeyFailure) throw new Error(window.aiSetKeyFailure);
        const provider = aiStatus.providers.find(p => p.id === args.provider);
        if (!provider) throw new Error(`"${args.provider}" is not a known AI provider.`);
        if (provider.local || provider.cli) throw new Error(`${provider.label} does not use an API key.`);
        if (args.key !== "" && !validKey(args.key)) throw new Error("API keys are printable ASCII text with no spaces, at most 512 bytes.");
        // The environment always wins in the real backend, so a stored env key's
        // `keySource` never changes just because the dialog saved or removed one.
        if (provider.keySource !== "env") provider.keySource = args.key ? "settings" : "none";
        return currentAiStatus();
      }
      if (command === "ai_ollama_models") {
        noUnknownArgs(args, []);
        if (window.ollamaModelsFailure) throw new Error(window.ollamaModelsFailure);
        return window.ollamaModelsOverride ?? ["qwen3.8:27b"];
      }
      if (command === "ai_test_provider") {
        noUnknownArgs(args, ["provider"]);
        // Same escape-hatch pattern as `window.aiStatusOverride`: none of the ai_*
        // handlers below can fail on their own, so a test that needs to see the
        // dialog's backend-error path sets this to the message it should show.
        if (window.aiTestFailure) throw new Error(window.aiTestFailure);
        const provider = aiStatus.providers.find(p => p.id === args.provider);
        const message = `Test succeeded using ${provider?.model || provider?.defaultModel || provider?.label || args.provider}.`;
        // Same deferral pattern as `deferRender`/`resolveRender`: lets a test hold
        // a test-provider reply open across a provider switch, to prove a late
        // reply never gets attributed to the provider shown afterward.
        if (window.deferTest) return new Promise(resolve => { window.resolveTest = () => resolve(message); });
        return message;
      }
      if (command === "create_generation_job") {
        const project = sessions.get(args.sessionId);
        const doc = project.documents.find(d => d.id === args.documentId);
        if (doc.revision !== args.expectedDocumentRevision) throw new Error("Document revision conflict");
        if (args.request.kind === "rework") {
          const req = args.request;
          if (req.provider !== aiStatus.provider) {
            throw new Error("AI settings changed after this request was created. Create the request again.");
          }
          // Mirrors the scope/target refusals `create_job_as` makes (runtime.rs),
          // so a request that never should have reached the backend (a widened
          // scope, a pruned-away target) fails here exactly like it would there.
          if (req.scope === "document") {
            if (req.sectionIds.length > 0 || req.diagramIds.length > 0) {
              throw new Error("A whole-document rework request must not select any blocks.");
            }
          } else if (req.scope === "selection") {
            const sectionIds = new Set(doc.sections.map(s => s.id));
            const diagramIds = new Set(doc.sections.flatMap(s => s.diagrams.map(g => g.id)));
            const hasTarget = req.sectionIds.some(id => sectionIds.has(id)) || req.diagramIds.some(id => diagramIds.has(id));
            if (!hasTarget) throw new Error("The selected blocks no longer exist.");
          } else {
            throw new Error("Unknown rework scope.");
          }
        }
        return putJob({ id: crypto.randomUUID(), projectId: project.id, documentId: doc.id,
          baseDocument: clone(doc), request: clone(args.request), status: "queued", sections: null,
          summary: null, error: null, createdAt: "2026-09-05 12:00:00" });
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
          } else if (job.request.kind === "rework") {
            // The mock never computes a merge itself: the test supplies the
            // exact post-merge sections (which ids stayed, which are new), the
            // same way it supplies the exact diagram content below.
            sections = raw.sections.map(s => ({ ...s, id: s.id || crypto.randomUUID(),
              diagrams: (s.diagrams ?? []).map(g => ({ ...g, id: g.id || crypto.randomUUID() })) }));
          } else {
            sections = clone(job.baseDocument.sections);
            let section = sections.find(s => s.id === job.request.sectionId);
            if (!section) { section = { id: crypto.randomUUID(), title: "Diagram", content: "", diagrams: [] }; sections.push(section); }
            const existing = section.diagrams.findIndex(g => g.id === job.request.diagramId);
            const diagram = { ...raw, id: job.request.diagramId ?? crypto.randomUUID() };
            if (existing < 0) section.diagrams.push(diagram); else section.diagrams[existing] = diagram;
          }
          const summary = job.request.kind === "rework" ? (raw.summary ?? null) : (latest.summary ?? null);
          resolve(putJob({ ...latest, status: "ready", sections, summary }));
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
      // A project is a folder. The mock treats every save as a Git commit of that folder;
      // like the backend it refuses a save over content it did not hand out (fingerprint).
      const commitId = index => (index + 1).toString(16).padEnd(40, "0");
      const fingerprint = project => `fp-${JSON.stringify(project).length}-${project.revision}`;
      // window.directoryPickResult overrides a folder pick (e.g. to null, for "dialog cancelled"),
      // the same escape-hatch pattern as window.confirmResult / window.loadFixture below.
      if (command === "plugin:dialog|open") {
        if (args.options?.directory) return "directoryPickResult" in window ? window.directoryPickResult : "fixture-project";
        return "legacy.archgen";
      }
      if (command === "save_project") {
        const stored = localStorage.getItem("test-saved-project");
        if (args.expectedFingerprint != null && (!stored || fingerprint(JSON.parse(stored)) !== args.expectedFingerprint)) throw new Error("Project changed on disk");
        localStorage.setItem("test-saved-project", JSON.stringify(args.project));
        const history = JSON.parse(localStorage.getItem("test-project-history") ?? "[]");
        if (!history.some(p => p.id === args.project.id && p.revision === args.project.revision)) history.push(args.project);
        localStorage.setItem("test-project-history", JSON.stringify(history));
        return fingerprint(args.project);
      }
      if (command === "list_project_history") {
        return JSON.parse(localStorage.getItem("test-project-history") ?? "[]").map((p, index) => ({ p, index })).filter(({ p }) => p.id === args.projectId)
          .reverse().slice(args.skip ?? 0, (args.skip ?? 0) + 50).map(({ p, index }) => ({ commit: commitId(index), savedAt: "2026-09-05T12:00:00+02:00", summary: `Save ${p.name}` }));
      }
      if (command === "load_project_revision") {
        const found = JSON.parse(localStorage.getItem("test-project-history") ?? "[]").find((p, index) => p.id === args.projectId && commitId(index) === args.commit);
        // Snapshots read from Git carry no revisions.
        return { ...found, revision: 0, documents: found.documents.map(d => ({ ...d, revision: 0 })) };
      }
      if (command === "load_project") {
        // A fixture stands for a folder that already exists on disk.
        if (window.loadFixture) localStorage.setItem("test-saved-project", JSON.stringify(window.loadFixture));
        const project = JSON.parse(localStorage.getItem("test-saved-project"));
        return { project, fingerprint: fingerprint(project) };
      }
      if (command === "import_legacy_project") return window.loadFixture;
      // Git (git.rs, release.rs). Argument names are checked like the ai_* commands:
      // a misnamed IPC argument is exactly what a mocked UI test must not hide.
      const git = window.gitFixture ?? {};
      if (command === "git_list_refs") {
        noUnknownArgs(args, ["path"]);
        if (git.refsError) throw new Error(git.refsError);
        return clone(git.refs ?? { root: "/repo", branch: "main", head: "c".repeat(40), refs: [
          { name: "v1.1.0", kind: "tag", commit: "b".repeat(40), date: "2026-09-10T10:00:00+02:00" },
          { name: "v1.0.0", kind: "tag", commit: "a".repeat(40), date: "2026-08-01T10:00:00+02:00" },
          { name: "main", kind: "branch", commit: "c".repeat(40), date: "2026-09-20T10:00:00+02:00" },
        ] });
      }
      if (command === "git_log_range") {
        noUnknownArgs(args, ["path", "from", "to", "subpath", "includeMerges"]);
        const commit = (n, subject, body = "") => ({ hash: String(n).repeat(40).slice(0, 40), shortHash: String(n).repeat(7), parents: 1, author: "Ján Novák", date: "2026-09-20T10:00:00+02:00", subject, body });
        return clone(git.range ?? { root: "/repo", from: "b".repeat(40), to: "c".repeat(40), truncated: false, commits: [
          commit(1, "feat(api): add refunds (#12)"), commit(2, "fix: rounding in totals"), commit(3, "chore: bump deps"),
        ] });
      }
      if (command === "git_project_status") {
        noUnknownArgs(args, ["path", "projectId"]);
        if (git.statusError) throw new Error(git.statusError);
        const committed = window.gitCommitted;
        return clone(git.status ?? { root: "/repo", branch: "main", head: "c".repeat(40), upstream: "origin/main", ahead: committed ? 1 : 0, behind: 0, truncated: false,
          changes: committed ? [] : [{ path: "archgen.json", status: "untracked" }, { path: "documents/untitled/document.json", status: "untracked" }] });
      }
      if (command === "git_commit_project") {
        noUnknownArgs(args, ["path", "projectId", "expectedFingerprint", "message"]);
        const stored = localStorage.getItem("test-saved-project");
        if (!stored || fingerprint(JSON.parse(stored)) !== args.expectedFingerprint) throw new Error("The project folder changed on disk since ArchGen last saved or opened it.");
        window.gitCommitted = args.message;
        return { commit: "d".repeat(40), summary: args.message.split("\n")[0], files: 2 };
      }
      if (command === "git_push_project") {
        noUnknownArgs(args, ["path", "projectId"]);
        window.gitPushed = true;
        return "Pushed main to origin.";
      }
      if (command === "list_release_templates") {
        noUnknownArgs(args, ["path", "projectId"]);
        return clone(window.releaseTemplates ?? []);
      }
      if (command === "save_release_template") {
        noUnknownArgs(args, ["path", "projectId", "name", "content", "overwrite"]);
        const file = `${args.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}.md`;
        window.releaseTemplates = [...(window.releaseTemplates ?? []).filter(t => t.file !== file), { file, content: args.content, error: null }];
        return file;
      }
      // site.rs: records the exported files for the test to inspect (window.exportedSite)
      // instead of writing to a real filesystem.
      if (command === "export_site") {
        noUnknownArgs(args, ["path", "files"]);
        if (window.exportSiteFailure) throw new Error(window.exportSiteFailure);
        window.exportedSite = { path: args.path, files: clone(args.files) };
        return { path: args.path, files: args.files.length, bytes: args.files.reduce((n, f) => n + f.content.length, 0) };
      }
      throw new Error(`Unexpected IPC: ${command}`);
    },
  };
}
