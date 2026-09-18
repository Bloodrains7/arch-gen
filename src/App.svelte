<script lang="ts">
  import Sidebar from "./lib/components/Sidebar.svelte";
  import Canvas from "./lib/components/Canvas.svelte";
  import PromptPanel from "./lib/components/PromptPanel.svelte";
  import Toolbar from "./lib/components/Toolbar.svelte";
  import Modal from "./lib/components/Modal.svelte";
  import ChangePreview from "./lib/components/ChangePreview.svelte";
  import { onMount } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { open, save, confirm } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { createProject, createDocument, updateDocument, captureTarget, applyGeneratedSections, parseProject, emptyHistory, recordChange, moveHistory, restoreSnapshot, sectionSource } from "./lib/project";
  import type { Project, Section } from "./lib/project";
  import { EditSession, jobCompatible } from "./lib/runtime";
  import type { GenerationJob, GenerationRequest } from "./lib/runtime";

  let project = $state(createProject());
  let projectPath = $state<string | null>(null);
  let savedRevision = $state(0);
  let fileBusy = $state(false);
  let status = $state("");
  let session = $state(0);
  const runtime = new EditSession(invoke);
  let runtimeReady = $state(false);
  let syncError = $state("");
  let syncing = $state(0);
  let editSerial = 0;
  let resolvingJob = $state(false);
  let preparingJobs = $state(0);
  let jobList = $state<GenerationJob[]>([]);
  let jobRefresh = 0;
  let jobs = $derived(preparingJobs + jobList.filter(j => j.status === "queued" || j.status === "running").length);
  let pending = $derived(jobList.filter(j => j.status === "ready" && j.sections).map(j => ({
    id: j.id, target: { projectId: j.projectId, documentId: j.documentId, revision: j.baseDocument.revision,
      template: j.baseDocument.template, language: j.baseDocument.language, name: j.baseDocument.name },
    before: j.baseDocument.sections, sections: j.sections!,
  })));
  let reviewingId = $state<string | null>(null);
  let reviewing = $derived(pending.find(item => item.id === reviewingId));
  let reviewCompatible = $derived(!!reviewing && !!jobList.find(j => j.id === reviewing.id && jobCompatible(project, j)) && !syncError);
  let undoHistory = $state(emptyHistory());
  let historyOpen = $state(false);
  let historyLoading = $state(false);
  let historyError = $state("");
  let historyEntries = $state<{ revision: number; savedAt: string }[]>([]);
  let historyMore = $state(false);
  let historySnapshot = $state<Project | null>(null);
  let historyBaseRevision = $state(0);
  let historyRequest = 0;
  let tabs = $derived(project.documents);
  let activeTabId = $derived(project.activeDocumentId);
  let activeTab = $derived(tabs.find(t => t.id === activeTabId)!);
  let dirty = $derived(project.revision !== savedRevision);
  let isGenerating = $state(false);

  function queueEdit(next: Project, invalidateDocuments = false) {
    const serial = ++editSerial;
    const startedSession = session;
    syncing++;
    runtime.edit(next, invalidateDocuments).then(acknowledged => {
      if (session === startedSession && serial === editSerial) project = parseProject(acknowledged);
    }).catch(error => { if (session === startedSession) syncError = `Edits could not be synchronized: ${error}`; })
      .finally(() => syncing--);
  }

  function commitProject(next: Project, label: string, group?: string, invalidateDocuments = false) {
    if (!runtimeReady || syncError || resolvingJob || next === project) return;
    undoHistory = recordChange(undoHistory, project, next, label, group);
    project = next;
    queueEdit(next, invalidateDocuments);
  }

  function undoRedo(direction: "undo" | "redo") {
    if (fileBusy || !runtimeReady || syncError || resolvingJob) return;
    const result = moveHistory(project, undoHistory, direction);
    if (!result) return;
    project = result.project; undoHistory = result.history;
    queueEdit(project, true);
    status = `${direction === "undo" ? "Undid" : "Redid"}: ${result.label}. Save to keep this version.`;
  }

  function renameProject(name: string) {
    if (name !== project.name) commitProject({ ...project, name, revision: project.revision + 1 }, "Rename project", "project-name");
  }

  function addTab() {
    const tab = createDocument();
    commitProject({ ...project, revision: project.revision + 1, documents: [...tabs, tab], activeDocumentId: tab.id }, "Add document");
  }

  async function closeTab(id: string) {
    if (tabs.length <= 1) return;
    const tab = tabs.find(t => t.id === id);
    if (!tab) return;
    const startedSession = session;
    const revision = tab.revision;
    if (!(await confirm(`Remove document "${tab.name}" from this project? You can restore it with Undo during this session.`, { title: "Remove document", kind: "warning" }))) return;
    if (!tabs.some(t => t.id === id) || tabs.length <= 1) return;
    if (startedSession !== session || tabs.find(t => t.id === id)?.revision !== revision) {
      status = "Document changed while confirming. Review it before removing."; return;
    }
    const idx = tabs.findIndex(t => t.id === id);
    const documents = tabs.filter(t => t.id !== id);
    commitProject({ ...project, revision: project.revision + 1, documents,
      activeDocumentId: activeTabId === id ? documents[Math.min(idx, documents.length - 1)].id : activeTabId }, `Remove ${tab.name}`);
  }

  function selectTab(id: string) {
    if (!runtimeReady || syncError || resolvingJob) return;
    if (id !== activeTabId) { project = { ...project, revision: project.revision + 1, activeDocumentId: id }; queueEdit(project); }
    undoHistory = { ...undoHistory, group: undefined };
  }

  function setTemplate(tmpl: string) {
    commitProject(updateDocument(project, activeTabId, { template: tmpl }), "Change template");
  }

  function setLanguage(lang: string) {
    commitProject(updateDocument(project, activeTabId, { language: lang }), "Change language");
  }

  function setSections(sections: Section[], group?: string) {
    commitProject(updateDocument(project, activeTabId, { sections }), group?.startsWith("title:") ? "Edit section title" : "Edit sections", group ? `${activeTabId}:${group}` : undefined);
  }

  function renameTab(id: string, name: string) {
    commitProject(updateDocument(project, id, { name }), "Rename document");
  }

  function beginUpdate(review = true) {
    const target = captureTarget(project);
    const startedSession = session;
    let finished = false;
    const finish = () => { finished = true; };
    return Object.assign((sections: Section[]) => {
      if (finished) return;
      finish();
      const result = startedSession === session ? applyGeneratedSections(project, target, sections) : null;
      if (result && !review) {
        commitProject(result, "Load template sections");
        status = `Updated ${target.name}. Save the project to keep changes.`;
      } else {
        status = "The document changed while loading the template. Select the template again to apply it.";
      }
    }, { finish });
  }

  async function acceptResult(id: string) {
    const item = pending.find(p => p.id === id);
    if (!item || resolvingJob || fileBusy || !runtimeReady || syncError) return;
    resolvingJob = true;
    try {
      const result = parseProject(await runtime.resolveJob("accept_generation_job", id));
      undoHistory = recordChange(undoHistory, project, result, `Accept AI changes: ${item.target.name}`);
      project = result; reviewingId = null;
      jobList = jobList.map(j => j.id === id ? { ...j, status: "accepted" } : j); jobRefresh++;
      status = `Updated ${item.target.name}. Save the project to keep changes.`;
    } catch (error) { status = `Acceptance failed: ${error}`; }
    finally { resolvingJob = false; }
  }

  async function discardResult(id: string) {
    try {
      upsertJob(await invoke<GenerationJob>("discard_generation_job", { jobId: id }));
      if (reviewingId === id) reviewingId = null;
      status = "Result discarded. Document content was not changed.";
    } catch (error) { status = `Discard failed: ${error}`; }
  }

  async function recoverResult(id: string) {
    const item = pending.find(p => p.id === id)!;
    if (!item || resolvingJob || fileBusy || !runtimeReady || syncError) return;
    resolvingJob = true;
    try {
      const result = parseProject(await runtime.resolveJob("recover_generation_job", id));
      undoHistory = recordChange(undoHistory, project, result, "Recover generated result");
      project = result; reviewingId = null;
      jobList = jobList.map(j => j.id === id ? { ...j, status: "recovered" } : j); jobRefresh++;
      status = "Recovered result as a new document. Save the project to keep it.";
    } catch (error) { status = `Recovery failed: ${error}`; }
    finally { resolvingJob = false; }
  }

  function upsertJob(job: GenerationJob) {
    if (job.projectId !== project.id) return;
    jobRefresh++;
    jobList = [job, ...jobList.filter(j => j.id !== job.id)];
  }

  async function refreshJobs() {
    const request = ++jobRefresh;
    const startedSession = session;
    const list = await invoke<GenerationJob[]>("list_generation_jobs", { projectId: project.id });
    if (request === jobRefresh && startedSession === session) jobList = list;
  }

  async function onGenerate(request: GenerationRequest) {
    if (!runtimeReady || syncError || resolvingJob) throw new Error("The edit session is not ready.");
    const documentId = activeTabId;
    const expectedDocumentRevision = activeTab.revision;
    const startedSession = session;
    preparingJobs++;
    let preparing = true;
    try {
      await runtime.flush();
      if (session !== startedSession || !runtimeReady) throw new Error("The project changed before generation started.");
      const job = await invoke<GenerationJob>("create_generation_job", { sessionId: runtime.id, documentId, expectedDocumentRevision, request });
      if (session === startedSession) upsertJob(job);
      preparingJobs--; preparing = false;
      const running = { ...job, status: "running" as const };
      if (session === startedSession) upsertJob(running);
      const result = await invoke<GenerationJob>("run_generation_job", { jobId: job.id });
      if (result.projectId === project.id) {
        upsertJob(result);
        status = result.status === "ready" ? `Result ready for ${result.baseDocument.name}. Review changes before applying them.`
          : `Generation ${result.status}${result.error ? `: ${result.error}` : "."}`;
      }
    } catch (error) {
      try { await refreshJobs(); } catch { /* Keep the original generation failure. */ }
      throw error;
    } finally { if (preparing) preparingJobs--; }
  }

  async function cancelJob(id: string) {
    try {
      upsertJob(await invoke<GenerationJob>("cancel_generation_job", { jobId: id }));
      status = "Job cancelled. Its result will not be applied; the current model request may still finish in the background.";
    } catch (error) { status = `Cancellation failed: ${error}`; }
  }

  async function initializeRuntime() {
    runtimeReady = false;
    try {
      project = parseProject(await runtime.open(parseProject($state.snapshot(project))));
      syncError = "";
      await refreshJobs();
      runtimeReady = true;
    } catch (error) { syncError = `Could not open the edit session: ${error}`; }
  }

  async function canLeave() {
    if (!dirty && jobs === 0 && !syncing) return true;
    const revision = project.revision;
    const currentSession = session;
    const pendingCount = pending.length;
    const jobCount = jobs;
    const accepted = await confirm("There are unsaved edits or an active generation. Leave without saving? Ready AI drafts remain in the local job registry.", { title: "Unsaved work", kind: "warning" });
    return accepted && revision === project.revision && currentSession === session && pendingCount === pending.length && jobCount === jobs;
  }

  async function newProject() {
    if (fileBusy) return;
    fileBusy = true;
    try {
      if (!(await canLeave())) return;
      runtimeReady = false;
      await runtime.flush();
      const fresh = parseProject(await runtime.open(createProject()));
      session++;
      project = fresh; projectPath = null; savedRevision = 0; jobList = [];
      undoHistory = emptyHistory(); reviewingId = null; closeHistory();
      status = "New project. Use Save project to choose a file.";
    } catch (error) { status = `Could not create project: ${error}`; }
    finally { fileBusy = false; runtimeReady = !!runtime.id; }
  }

  async function saveProject(saveAs = false) {
    if (fileBusy) return;
    fileBusy = true;
    try {
      const path = saveAs || !projectPath ? await save({ title: "Save ArchGen project (choose a new filename)", defaultPath: "project.archgen", filters: [{ name: "ArchGen project", extensions: ["archgen"] }] }) : projectPath;
      if (!path) return;
      const snapshot = parseProject(await runtime.flush());
      undoHistory = { ...undoHistory, group: undefined };
      const expectedRevision = !saveAs && path === projectPath ? savedRevision : null;
      await invoke("save_project", { path, project: snapshot, expectedRevision });
      projectPath = path; savedRevision = snapshot.revision;
      status = `Saved ${path}${project.revision !== snapshot.revision ? " — newer edits still need saving." : ""}`;
    } catch (error) { status = `Save failed: ${error}`; }
    finally { fileBusy = false; }
  }

  async function openProject() {
    if (fileBusy) return;
    fileBusy = true;
    try {
      if (!(await canLeave())) return;
      const revision = project.revision;
      const pendingCount = pending.length;
      const path = await open({ title: "Open ArchGen project", multiple: false, filters: [{ name: "ArchGen project", extensions: ["archgen"] }] });
      if (!path || typeof path !== "string") return;
      const loaded = parseProject(await invoke<unknown>("load_project", { path }));
      if (project.revision !== revision || pending.length !== pendingCount) throw new Error("The current project changed while opening. Save it and try again.");
      runtimeReady = false;
      await runtime.flush();
      const acknowledged = parseProject(await runtime.open(loaded));
      session++; project = acknowledged; projectPath = path; savedRevision = loaded.revision; jobList = [];
      await refreshJobs();
      undoHistory = emptyHistory(); reviewingId = null; closeHistory();
      status = `Opened ${path}`;
    } catch (error) { status = `Open failed: ${error}`; }
    finally { fileBusy = false; runtimeReady = !!runtime.id; }
  }

  function closeHistory() {
    historyOpen = false; historyRequest++; historyLoading = false; historySnapshot = null;
  }

  async function loadHistory(more = false) {
    if (!projectPath || fileBusy) return;
    historyOpen = true; historyLoading = true; historyError = "";
    const request = ++historyRequest;
    const startedSession = session;
    if (!more) { historyEntries = []; historySnapshot = null; }
    try {
      const entries = await invoke<{ revision: number; savedAt: string }[]>("list_project_history", {
        path: projectPath, projectId: project.id, beforeRevision: more ? historyEntries.at(-1)?.revision : null,
      });
      if (request !== historyRequest || startedSession !== session) return;
      historyEntries = more ? [...historyEntries, ...entries] : entries;
      historyMore = entries.length === 50;
    } catch (error) { if (request === historyRequest) historyError = `History failed: ${error}`; }
    finally { if (request === historyRequest) historyLoading = false; }
  }

  async function previewHistory(revision: number) {
    if (!projectPath) return;
    historyLoading = true; historyError = ""; historySnapshot = null;
    const request = ++historyRequest;
    const startedSession = session;
    const base = project.revision;
    try {
      const snapshot = parseProject(await invoke("load_project_revision", { path: projectPath, projectId: project.id, revision }));
      if (request !== historyRequest || startedSession !== session) return;
      if (snapshot.id !== project.id || snapshot.revision !== revision) throw new Error("Unexpected project revision.");
      historySnapshot = snapshot; historyBaseRevision = base;
    } catch (error) { if (request === historyRequest) historyError = `Could not preview revision: ${error}`; }
    finally { if (request === historyRequest) historyLoading = false; }
  }

  function restoreHistory() {
    if (!historySnapshot || historyLoading || project.revision !== historyBaseRevision) return;
    try {
      const revision = historySnapshot.revision;
      commitProject(restoreSnapshot(project, historySnapshot), `Restore saved revision ${revision}`, undefined, true);
      closeHistory();
      status = `Restored saved revision ${revision} as a new working revision. Save to keep it; Undo returns to your previous work.`;
    } catch (error) { historyError = `Restore failed: ${error}`; }
  }

  onMount(() => {
    void initializeRuntime();
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWindow().onCloseRequested(async event => {
      if (fileBusy || dirty || jobs || syncing || resolvingJob) {
        event.preventDefault();
        if (!fileBusy && await canLeave()) await getCurrentWindow().destroy();
      }
    }).then(stop => { if (disposed) stop(); else unlisten = stop; });
    return () => { disposed = true; unlisten?.(); };
  });

  function beforeUnload(event: BeforeUnloadEvent) {
    if (!isTauri() && (dirty || jobs || fileBusy || syncing || resolvingJob)) {
      event.preventDefault(); event.returnValue = "";
    }
  }
</script>

<svelte:window onbeforeunload={beforeUnload} />
{#if syncError}
  <div class="runtime-error" role="alert">{syncError} Your visible edits are preserved.
    <button onclick={initializeRuntime}>Retry synchronization</button>
  </div>
{:else if !runtimeReady}<div class="runtime-error" role="status">Opening project edit session…</div>{/if}
<div class="layout" inert={!runtimeReady || !!syncError || resolvingJob}>
  <Sidebar
    selectedTemplate={activeTab.template}
    selectedLanguage={activeTab.language}
    sections={activeTab.sections}
    onTemplateChange={(tmpl: string) => setTemplate(tmpl)}
    onLanguageChange={(l: string) => setLanguage(l)}
    onSectionsChange={setSections}
    {beginUpdate}
  />
  <div class="main">
    <div class="project-bar">
      <input aria-label="Project name" value={project.name} oninput={(e) => renameProject(e.currentTarget.value)} />
      <span>{syncing ? "Synchronizing edits…" : dirty ? "Unsaved changes" : projectPath ? "Saved" : "New project"}</span>
      <button disabled={fileBusy} onclick={newProject}>New project</button>
      <button disabled={fileBusy} onclick={openProject}>Open project</button>
      <button disabled={fileBusy} onclick={() => saveProject()}>Save project</button>
      <button disabled={fileBusy} onclick={() => saveProject(true)}>Save as…</button>
    </div>
    <div class="history-bar">
      <button disabled={fileBusy || !undoHistory.past.length} onclick={() => undoRedo("undo")} title={undoHistory.past.at(-1)?.label}>Undo</button>
      <button disabled={fileBusy || !undoHistory.future.length} onclick={() => undoRedo("redo")} title={undoHistory.future.at(-1)?.label}>Redo</button>
      <button disabled={fileBusy || !projectPath} onclick={() => loadHistory()}>Saved history</button>
      <span>Undo/Redo: current session · Saved history: versions saved to this file</span>
    </div>
    {#if status}<div class="project-status" role="status">{status}</div>{/if}
    <div class="pending-results">
    {#each jobList.filter(j => ["queued", "running", "failed", "interrupted", "cancelled"].includes(j.status)) as job (job.id)}
      <div class="project-status">
        {job.baseDocument.name}: {job.status}{job.error ? ` — ${job.error}` : ""}
        {#if job.status === "queued" || job.status === "running"}
          <button onclick={() => cancelJob(job.id)}>Cancel generation</button>
        {:else if job.status !== "cancelled"}<button onclick={() => discardResult(job.id)}>Dismiss job</button>{/if}
      </div>
    {/each}
    {#each pending as item (item.id)}
      <div class="project-status">
        Result for {item.target.name} is waiting for review.
        <button onclick={() => reviewingId = item.id}>Review changes</button>
        <button onclick={() => recoverResult(item.id)}>Recover as new document</button>
        <button onclick={() => discardResult(item.id)}>Discard result</button>
      </div>
    {/each}
    </div>
    <div class="tab-bar">
      <div class="tabs-scroll">
        {#each tabs as tab (tab.id)}
          <div
            class="tab"
            class:active={tab.id === activeTabId}
            onclick={() => selectTab(tab.id)}
            onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectTab(tab.id); } }}
            ondblclick={() => {
              const name = prompt("Rename tab:", tab.name);
              if (name) renameTab(tab.id, name);
            }}
            role="tab"
            tabindex="0"
          >
            <span class="tab-name">{tab.name}</span>
            <span class="tab-template">{tab.template}</span>
            {#if tabs.length > 1}
              <button
                class="tab-close"
                onclick={(e) => { e.stopPropagation(); closeTab(tab.id); }}
              >×</button>
            {/if}
          </div>
        {/each}
      </div>
      <button class="tab-add" onclick={addTab} title="New document">+</button>
    </div>
    <Toolbar selectedTemplate={activeTab.template} sections={activeTab.sections} />
    {#key activeTab.id}
    <Canvas
      sections={activeTab.sections}
      {isGenerating}
      selectedLanguage={activeTab.language}
      onSectionsChange={setSections}
      {onGenerate}
      onError={(message: string) => status = message}
    />
    {/key}
    <PromptPanel
      sections={activeTab.sections}
      bind:isGenerating
      selectedTemplate={activeTab.template}
      selectedLanguage={activeTab.language}
      {onGenerate}
    />
  </div>
</div>

{#if reviewing}
  <Modal title={`Review changes — ${reviewing.target.name}`} onClose={() => reviewingId = null}>
    <p>The proposal replaces this document's sections, including the diagram source shown below. Nothing changes until you accept.</p>
    {#if !reviewCompatible}
      <p class="conflict" role="alert">The original document changed or closed. Acceptance is blocked. Recover as a new document to keep this result.</p>
      <details><summary>Current document source</summary><pre class="current-source">{project.documents.find(d => d.id === reviewing!.target.documentId)?.sections.map(sectionSource).join("\n\n") ?? "Document is no longer open in this project."}</pre></details>
    {/if}
    <ChangePreview before={reviewing.before} after={reviewing.sections} />
    <div class="review-actions">
      <button disabled={!reviewCompatible || resolvingJob || fileBusy || !runtimeReady} onclick={() => acceptResult(reviewing!.id)}>Accept changes</button>
      <button disabled={resolvingJob || fileBusy || !runtimeReady} onclick={() => recoverResult(reviewing!.id)}>Recover as new document</button>
      <button disabled={resolvingJob} onclick={() => discardResult(reviewing!.id)}>Discard result</button>
    </div>
  </Modal>
{/if}

{#if historyOpen}
  <Modal title="Saved project history" onClose={closeHistory}>
    <p>These are saved versions of the whole project. Restore creates a new working revision; save it to keep it. Your current work remains available through Undo.</p>
    {#if historyError}<p role="alert">{historyError}</p>{/if}
    {#if historyLoading}<p role="status">Loading history…</p>{/if}
    <div class="revision-list">
      {#each historyEntries as entry (entry.revision)}
        <button disabled={historyLoading} onclick={() => previewHistory(entry.revision)}>Revision {entry.revision} · {entry.savedAt} UTC</button>
      {/each}
      {#if historyMore}<button disabled={historyLoading} onclick={() => loadHistory(true)}>Load older versions</button>{/if}
    </div>
    {#if historySnapshot}
      <h3>Revision {historySnapshot.revision}: {historySnapshot.name}</h3>
      <p>Project name: {project.name} → {historySnapshot.name}</p>
      {#each historySnapshot.documents as document (document.id)}
        {@const current = project.documents.find(d => d.id === document.id)}
        <h4>{current ? "Restore document" : "Reintroduce document"}: {document.name}</h4>
        <p>Name: {current?.name ?? "(absent)"} → {document.name} · Template: {current?.template ?? "—"} → {document.template} · Language: {current?.language ?? "—"} → {document.language}</p>
        <ChangePreview before={current?.sections ?? []} after={document.sections} />
      {/each}
      {#each project.documents.filter(d => !historySnapshot!.documents.some(old => old.id === d.id)) as removed (removed.id)}
        <h4>Remove document: {removed.name}</h4>
        <ChangePreview before={removed.sections} after={[]} />
      {/each}
      {#if project.revision !== historyBaseRevision}<p role="alert">The project changed during preview. Select the revision again before restoring.</p>{/if}
      <div class="review-actions"><button disabled={historyLoading || project.revision !== historyBaseRevision} onclick={restoreHistory}>Restore this version</button></div>
    {:else if !historyLoading && !historyEntries.length && !historyError}
      <p>No saved versions found.</p>
    {/if}
  </Modal>
{/if}

<style>
  .runtime-error { position: fixed; top: 0; left: 0; right: 0; z-index: 10; padding: 16px; background: var(--bg-secondary); color: var(--text-primary); }
  .history-bar { display: flex; align-items: center; gap: 8px; padding: 8px 20px; background: var(--bg-secondary); }
  .history-bar span { font-size: 12px; color: var(--text-muted); }
  .history-bar button, .review-actions button, .revision-list button { padding: 7px 12px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; }
  .review-actions { display: flex; gap: 10px; position: sticky; bottom: -20px; padding: 16px 0; background: var(--bg-secondary); }
  .revision-list { display: flex; flex-wrap: wrap; gap: 8px; max-height: 180px; overflow-y: auto; }
  .pending-results { flex-shrink: 0; max-height: 150px; overflow-y: auto; }
  .conflict { color: var(--danger); }
  .current-source { white-space: pre-wrap; max-height: 250px; overflow: auto; }
  .project-bar { display: flex; gap: 10px; align-items: center; padding: 10px 20px; background: var(--bg-secondary); }
  .project-bar input { min-width: 100px; flex: 1; padding: 6px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); }
  .project-bar span { font-size: 12px; color: var(--text-muted); }
  .project-bar button, .project-status button { padding: 6px 10px; background: var(--bg-tertiary); color: var(--text-primary); border: 1px solid var(--border); border-radius: 4px; }
  .project-status { padding: 8px 20px; font-size: 12px; overflow-wrap: anywhere; background: var(--bg-secondary); }
  .layout {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .tab-bar {
    display: flex;
    align-items: stretch;
    background: var(--bg-primary);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    min-height: 38px;
  }

  .tabs-scroll {
    display: flex;
    overflow-x: auto;
    flex: 1;
    scrollbar-width: none;
  }

  .tabs-scroll::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    background: transparent;
    border: none;
    border-right: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
    transition: all 0.15s;
    cursor: pointer;
    position: relative;
  }

  .tab:hover {
    background: var(--bg-secondary);
    color: var(--text-secondary);
  }

  .tab.active {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-bottom: 2px solid var(--accent);
  }

  .tab-name {
    font-weight: 500;
  }

  .tab-template {
    font-size: 10px;
    color: var(--text-muted);
    background: var(--bg-tertiary);
    padding: 1px 5px;
    border-radius: 3px;
  }

  .tab-close {
    background: transparent;
    color: var(--text-muted);
    font-size: 14px;
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 3px;
    padding: 0;
    transition: all 0.1s;
  }

  .tab-close:hover {
    background: var(--danger);
    color: white;
  }

  .tab-add {
    padding: 8px 14px;
    background: transparent;
    color: var(--text-muted);
    font-size: 16px;
    transition: all 0.15s;
    flex-shrink: 0;
  }

  .tab-add:hover {
    color: var(--accent);
    background: var(--bg-secondary);
  }
</style>
