<script lang="ts">
  import Sidebar from "./lib/components/Sidebar.svelte";
  import Canvas from "./lib/components/Canvas.svelte";
  import PromptPanel from "./lib/components/PromptPanel.svelte";
  import Toolbar from "./lib/components/Toolbar.svelte";
  import Modal from "./lib/components/Modal.svelte";
  import ChangePreview from "./lib/components/ChangePreview.svelte";
  import AiSettings from "./lib/components/AiSettings.svelte";
  import GitStatusBar from "./lib/components/GitStatusBar.svelte";
  import ReleaseNotesDialog from "./lib/components/ReleaseNotesDialog.svelte";
  import { onMount, tick } from "svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { createProject, createDocument, updateDocument, normalizeSections, parseProject, emptyHistory, recordChange, moveHistory, restoreSnapshot, sectionSource } from "./lib/project";
  import { applyTemplateStructure, findTemplate, isBlankSection } from "./lib/templates";
  import type { Diagram, Project, ProjectDocument, Section } from "./lib/project";
  import { buildSite, renderDiagrams } from "./lib/site-export";
  import type { DiagramRenderer } from "./lib/site-export";
  import { sanitizeSvg } from "./lib/local-svg";
  import { EditSession, jobCompatible } from "./lib/runtime";
  import type { GenerationJob, GenerationRequest } from "./lib/runtime";
  import { fetchAiStatus } from "./lib/ai";
  import type { AiStatus } from "./lib/ai";
  import { emptySelection, pruneSelection, toggleSection, toggleDiagram } from "./lib/selection";
  import type { BlockSelection } from "./lib/selection";

  let project = $state(createProject());
  let projectPath = $state<string | null>(null);
  let savedRevision = $state(0);
  // Identifies the on-disk content of the last load or save; a save over anything else is refused.
  let savedFingerprint = $state<string | null>(null);
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
    before: j.baseDocument.sections, sections: j.sections!, baseDocument: j.baseDocument, request: j.request, summary: j.summary,
  })));
  let reviewingId = $state<string | null>(null);
  let reviewing = $derived(pending.find(item => item.id === reviewingId));
  let reviewCompatible = $derived(!!reviewing && !!jobList.find(j => j.id === reviewing.id && jobCompatible(project, j)) && !syncError);
  let undoHistory = $state(emptyHistory());
  let historyOpen = $state(false);
  let historyLoading = $state(false);
  let historyError = $state("");
  let historyEntries = $state<HistoryEntry[]>([]);
  let historyMore = $state(false);
  let historySnapshot = $state<Project | null>(null);
  let historyBaseRevision = $state(0);
  let historyRequest = 0;
  let tabs = $derived(project.documents);
  let activeTabId = $derived(project.activeDocumentId);
  let activeTab = $derived(tabs.find(t => t.id === activeTabId)!);
  type HistoryEntry = { commit: string; savedAt: string; summary: string };
  let historyCommit = $state<HistoryEntry | null>(null);
  let dirty = $derived(project.revision !== savedRevision);
  let isGenerating = $state(false);
  let aiStatus = $state<AiStatus | null>(null);
  let aiStatusError = $state("");
  let aiSettingsOpen = $state(false);
  let releaseNotesOpen = $state(false);
  // The element to return focus to once the dialog's `{#if}` block has actually
  // left the DOM — captured at open time, since by then it is always the "AI
  // settings" button that has focus (settings-dialog-and-e2e-quality#11).
  let aiSettingsOpener = $state<HTMLElement | null>(null);
  // Tagged with the session and document it was made in, so switching tabs, undo/redo,
  // accepted AI results, Git restore and open/new/import all fall back to empty instead
  // of carrying a stale selection into another document. `selection` also drops any ID
  // that no longer exists in the active tab (e.g. a section just removed).
  // `includeContext` lives on the same tagged object (design doc §9): it carries over
  // while the user keeps ticking within one document, but never into another one.
  let rawSelection = $state<BlockSelection & { session: number; documentId: string; includeContext: boolean }>(
    { session: -1, documentId: "", sectionIds: [], diagramIds: [], includeContext: false },
  );
  let selectionMatch = $derived(rawSelection.session === session && rawSelection.documentId === activeTabId);
  let selection = $derived(selectionMatch ? pruneSelection(rawSelection, activeTab.sections) : emptySelection());
  let includeContext = $derived(selectionMatch ? rawSelection.includeContext : false);
  // Whole-document rework is armed only by a Rework click with nothing ticked
  // (design doc §9); tagged the same way, so it drops by itself on a tab switch
  // or session change, and is dropped explicitly by every toggle, "Clear
  // selection", and a created rework request (see the functions below).
  let documentScope = $state({ session: -1, documentId: "" });
  let documentScopeArmed = $derived(documentScope.session === session && documentScope.documentId === activeTabId);

  function armDocumentScope() {
    documentScope = { session, documentId: activeTabId };
  }

  async function refreshAiStatus() {
    try { aiStatus = await fetchAiStatus(invoke); aiStatusError = ""; }
    catch (error) { aiStatus = null; aiStatusError = `Could not load AI status: ${error}`; }
  }

  function toggleSectionSelection(id: string) {
    documentScope = { session: -1, documentId: "" };
    rawSelection = { session, documentId: activeTabId, includeContext, ...toggleSection(selection, id) };
  }

  function toggleDiagramSelection(sectionId: string, diagramId: string) {
    documentScope = { session: -1, documentId: "" };
    rawSelection = { session, documentId: activeTabId, includeContext, ...toggleDiagram(selection, sectionId, diagramId) };
  }

  function clearSelection() {
    documentScope = { session: -1, documentId: "" };
    rawSelection = { session, documentId: activeTabId, includeContext: false, ...emptySelection() };
  }

  function setIncludeContext(value: boolean) {
    rawSelection = { session, documentId: activeTabId, sectionIds: selection.sectionIds, diagramIds: selection.diagramIds, includeContext: value };
  }

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

  // Applying a template never loses content: matching sections are kept, other
  // sections with content move after the template's, and only blank ones go.
  function applyTemplate(id: string) {
    const template = findTemplate(id);
    if (!template) return;
    const sections = applyTemplateStructure(activeTab.sections, template);
    const kept = activeTab.sections.filter(s => sections.includes(s) && !isBlankSection(s)).length;
    commitProject(updateDocument(project, activeTabId, { template: id, sections }), `Apply ${template.name} template`);
    status = kept ? `Applied the ${template.name} structure; ${kept} existing section${kept === 1 ? " was" : "s were"} kept. Undo reverts it.` : `Applied the ${template.name} structure.`;
  }

  function createReleaseDocument(name: string, sections: Section[]) {
    const created = { ...createDocument(name), template: "release-notes", language: activeTab.language, sections: normalizeSections(sections) };
    commitProject({ ...project, revision: project.revision + 1, documents: [...tabs, created], activeDocumentId: created.id }, "Add release notes");
    status = `Created document "${name}" from Git. Save the project to keep it.`;
  }

  function insertReleaseSections(sections: Section[]) {
    commitProject(updateDocument(project, activeTabId, { sections: [...sections, ...activeTab.sections] }), "Insert release notes");
    status = `Inserted release notes at the top of ${activeTab.name}. Save the project to keep them.`;
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

  // Provider/model come from the job's own `request`, never from the current
  // `aiStatus`: a rework job always ran with the provider it was created for.
  function reworkLabel(request: GenerationRequest): string {
    return request.kind === "rework" ? ` · ${request.provider}${request.model ? ` (${request.model})` : ""}` : "";
  }

  function reworkTargets(request: GenerationRequest, baseDocument: ProjectDocument): string {
    if (request.kind !== "rework") return "";
    if (request.scope === "document") return "the whole document";
    const sectionTitles = request.sectionIds.map(id => baseDocument.sections.find(s => s.id === id)?.title || "untitled");
    const diagramTitles = request.diagramIds.map(id => {
      for (const section of baseDocument.sections) {
        const diagram = section.diagrams.find(d => d.id === id);
        if (diagram) return `${diagram.diagram_type} in ${section.title}`;
      }
      return id;
    });
    return [...sectionTitles, ...diagramTitles].join(", ") || "(no blocks)";
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
      if (session === startedSession) {
        upsertJob(job);
        // A created rework request has spent the whole-document arm and the
        // context tick (design doc §9): leaving either set would let the next
        // empty selection, or the next tick, silently reuse a consent the
        // user already gave for a different request.
        if (request.kind === "rework") {
          documentScope = { session: -1, documentId: "" };
          rawSelection = { ...rawSelection, includeContext: false };
        }
      }
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
      // A rejected rework creation usually means AI settings changed underneath the
      // request (a different provider configured, a key removed); refresh so the
      // panel reflects what is actually configured now instead of a stale status.
      if (request.kind === "rework") await refreshAiStatus();
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
      project = fresh; projectPath = null; savedRevision = 0; savedFingerprint = null; jobList = [];
      // A rework still running for the project just left must not leave the new,
      // unrelated project stuck showing "Generating..." until it settles.
      isGenerating = false;
      undoHistory = emptyHistory(); reviewingId = null; closeHistory();
      status = "New project. Use Save project to choose a folder.";
    } catch (error) { status = `Could not create project: ${error}`; }
    finally { fileBusy = false; runtimeReady = !!runtime.id; }
  }

  async function saveProject(saveAs = false) {
    if (fileBusy) return;
    fileBusy = true;
    try {
      const path = saveAs || !projectPath ? await open({ title: "Choose a folder for the ArchGen project (no existing project inside)", directory: true, multiple: false }) : projectPath;
      if (!path || typeof path !== "string") return;
      const snapshot = parseProject(await runtime.flush());
      undoHistory = { ...undoHistory, group: undefined };
      const expectedFingerprint = !saveAs && path === projectPath ? savedFingerprint : null;
      savedFingerprint = await invoke<string>("save_project", { path, project: snapshot, expectedFingerprint });
      projectPath = path; savedRevision = snapshot.revision;
      status = `Saved ${path}${project.revision !== snapshot.revision ? " — newer edits still need saving." : ""}`;
    } catch (error) { status = `Save failed: ${error}`; }
    finally { fileBusy = false; }
  }

  const today = () => new Date().toLocaleDateString("sv-SE");

  // Exports the project as it is now, unsaved edits included (dirty says so in the status
  // line). PlantUML renders locally through the same private renderer Toolbar's HTML export
  // uses; this is the one place another local renderer (Mermaid) gets plugged in later —
  // everything else stays as fenced source in the page, exactly like the single-document export.
  const renderSiteDiagram: DiagramRenderer = async (diagram: Diagram) => {
    if (diagram.format !== "plantuml") return null;
    try { return sanitizeSvg(await invoke<string>("render_local_diagram", { content: diagram.content })); }
    catch { return null; }
  };

  async function exportSite() {
    if (fileBusy) return;
    fileBusy = true;
    try {
      const path = await open({ title: "Choose an empty folder for the documentation site", directory: true, multiple: false });
      if (!path || typeof path !== "string") return;
      const snapshot: Project = $state.snapshot(project);
      const rendered = await renderDiagrams(snapshot, renderSiteDiagram);
      const { files, stats } = buildSite(snapshot, { projectName: snapshot.name, date: today() }, rendered);
      await invoke("export_site", { path, files });
      const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;
      status = `Exported ${plural(stats.documents, "document")} to ${path} — ${plural(stats.diagramsRendered, "diagram")} as SVG, `
        + `${plural(stats.diagramsAsSource, "diagram")} as source${dirty ? " (unsaved edits included)" : ""}`;
    } catch (error) { status = `Site export failed: ${error}`; }
    finally { fileBusy = false; }
  }

  async function openProject(legacy = false) {
    if (fileBusy) return;
    fileBusy = true;
    try {
      if (!(await canLeave())) return;
      const revision = project.revision;
      const pendingCount = pending.length;
      const path = legacy
        ? await open({ title: "Import a former .archgen project file", multiple: false, filters: [{ name: "ArchGen project file", extensions: ["archgen"] }] })
        : await open({ title: "Open ArchGen project folder", directory: true, multiple: false });
      if (!path || typeof path !== "string") return;
      const disk = legacy
        ? { project: await invoke<unknown>("import_legacy_project", { path }), fingerprint: null }
        : await invoke<{ project: unknown; fingerprint: string }>("load_project", { path });
      const loaded = parseProject(disk.project);
      if (project.revision !== revision || pending.length !== pendingCount) throw new Error("The current project changed while opening. Save it and try again.");
      runtimeReady = false;
      await runtime.flush();
      const acknowledged = parseProject(await runtime.open(loaded));
      // An imported file is an unsaved project until a folder is chosen for it.
      session++; project = acknowledged; projectPath = legacy ? null : path; savedRevision = legacy ? -1 : loaded.revision;
      savedFingerprint = disk.fingerprint; jobList = [];
      // Same reasoning as `newProject`: the project being left behind may still
      // have a rework in flight, and that must not block the one just opened.
      isGenerating = false;
      await refreshJobs();
      undoHistory = emptyHistory(); reviewingId = null; closeHistory();
      status = legacy ? `Imported ${path}. Use Save project to choose a folder for it.` : `Opened ${path}`;
    } catch (error) { status = `Open failed: ${error}`; }
    finally { fileBusy = false; runtimeReady = !!runtime.id; }
  }

  function closeHistory() {
    historyOpen = false; historyRequest++; historyLoading = false; historySnapshot = null; historyCommit = null;
  }

  async function loadHistory(more = false) {
    if (!projectPath || fileBusy) return;
    historyOpen = true; historyLoading = true; historyError = "";
    const request = ++historyRequest;
    const startedSession = session;
    if (!more) { historyEntries = []; historySnapshot = null; }
    try {
      const entries = await invoke<HistoryEntry[]>("list_project_history", {
        path: projectPath, projectId: project.id, skip: more ? historyEntries.length : 0,
      });
      if (request !== historyRequest || startedSession !== session) return;
      historyEntries = more ? [...historyEntries, ...entries] : entries;
      historyMore = entries.length === 50;
    } catch (error) { if (request === historyRequest) historyError = `History failed: ${error}`; }
    finally { if (request === historyRequest) historyLoading = false; }
  }

  async function previewHistory(entry: HistoryEntry) {
    if (!projectPath) return;
    historyLoading = true; historyError = ""; historySnapshot = null; historyCommit = null;
    const request = ++historyRequest;
    const startedSession = session;
    const base = project.revision;
    try {
      const snapshot = parseProject(await invoke("load_project_revision", { path: projectPath, projectId: project.id, commit: entry.commit }));
      if (request !== historyRequest || startedSession !== session) return;
      if (snapshot.id !== project.id) throw new Error("This commit holds another project.");
      historySnapshot = snapshot; historyCommit = entry; historyBaseRevision = base;
    } catch (error) { if (request === historyRequest) historyError = `Could not preview commit: ${error}`; }
    finally { if (request === historyRequest) historyLoading = false; }
  }

  function restoreHistory() {
    if (!historySnapshot || !historyCommit || historyLoading || project.revision !== historyBaseRevision) return;
    try {
      const commit = historyCommit.commit.slice(0, 8);
      // A snapshot read from Git carries no session state; keep the open tab when it still exists.
      const snapshot = { ...historySnapshot, activeDocumentId: historySnapshot.documents.some(d => d.id === activeTabId) ? activeTabId : historySnapshot.activeDocumentId };
      commitProject(restoreSnapshot(project, snapshot), `Restore commit ${commit}`, undefined, true);
      closeHistory();
      status = `Restored commit ${commit} as a new working revision. Save to keep it; Undo returns to your previous work.`;
    } catch (error) { historyError = `Restore failed: ${error}`; }
  }

  onMount(() => {
    void initializeRuntime();
    void refreshAiStatus();
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
    onApplyTemplate={applyTemplate}
    onLanguageChange={(l: string) => setLanguage(l)}
  />
  <div class="main">
    <div class="project-bar">
      <input aria-label="Project name" value={project.name} oninput={(e) => renameProject(e.currentTarget.value)} />
      <span>{syncing ? "Synchronizing edits…" : dirty ? "Unsaved changes" : projectPath ? "Saved" : "New project"}</span>
      <button disabled={fileBusy} onclick={newProject}>New project</button>
      <button disabled={fileBusy} onclick={() => openProject()}>Open project</button>
      <button disabled={fileBusy} onclick={() => saveProject()}>Save project</button>
      <button disabled={fileBusy} onclick={() => saveProject(true)}>Save as…</button>
      <button disabled={fileBusy} onclick={exportSite} title="Publish every document as a Markdown site (DocFX / MkDocs / Azure DevOps wiki / GitHub)">Export site…</button>
      <button disabled={fileBusy} onclick={() => openProject(true)} title="Open a project saved by an older ArchGen as a single .archgen file">Import .archgen</button>
    </div>
    <div class="history-bar">
      <button disabled={fileBusy || !undoHistory.past.length} onclick={() => undoRedo("undo")} title={undoHistory.past.at(-1)?.label}>Undo</button>
      <button disabled={fileBusy || !undoHistory.future.length} onclick={() => undoRedo("redo")} title={undoHistory.future.at(-1)?.label}>Redo</button>
      <button disabled={fileBusy || !projectPath} onclick={() => loadHistory()}>Git history</button>
      <button disabled={fileBusy} onclick={() => releaseNotesOpen = true}>Release notes…</button>
      <span class="spacer"></span>
      <GitStatusBar {projectPath} projectId={project.id} projectName={project.name} fingerprint={savedFingerprint}
        {dirty} busy={fileBusy || syncing > 0} onStatus={(message: string) => status = message} />
    </div>
    {#if status}<div class="project-status" role="status">{status}</div>{/if}
    <div class="pending-results">
    {#each jobList.filter(j => ["queued", "running", "failed", "interrupted", "cancelled"].includes(j.status)) as job (job.id)}
      <div class="project-status">
        {job.baseDocument.name}: {job.status}{reworkLabel(job.request)}{job.error ? ` — ${job.error}` : ""}
        {#if job.status === "queued" || job.status === "running"}
          <button onclick={() => cancelJob(job.id)}>Cancel generation</button>
        {:else if job.status !== "cancelled"}<button onclick={() => discardResult(job.id)}>Dismiss job</button>{/if}
      </div>
    {/each}
    {#each pending as item (item.id)}
      <div class="project-status">
        Result for {item.target.name} is waiting for review.{reworkLabel(item.request)}{item.summary ? ` — ${item.summary}` : ""}
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
    <Toolbar document={activeTab} projectName={project.name} />
    {#key activeTab.id}
    <Canvas
      sections={activeTab.sections}
      documentName={activeTab.name}
      {isGenerating}
      selectedLanguage={activeTab.language}
      {selection}
      onSectionsChange={setSections}
      {onGenerate}
      onError={(message: string) => status = message}
      onToggleSection={toggleSectionSelection}
      onToggleDiagram={toggleDiagramSelection}
    />
    {/key}
    <PromptPanel
      sections={activeTab.sections}
      bind:isGenerating
      selectedTemplate={activeTab.template}
      selectedLanguage={activeTab.language}
      {selection}
      {includeContext}
      {aiStatus}
      {aiStatusError}
      {documentScopeArmed}
      {session}
      {onGenerate}
      onClearSelection={clearSelection}
      onOpenAiSettings={() => {
        aiSettingsOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        aiSettingsOpen = true;
        void refreshAiStatus();
      }}
      onArmDocumentScope={armDocumentScope}
      onIncludeContextChange={setIncludeContext}
    />
  </div>
</div>

{#if aiSettingsOpen}
  <AiSettings
    {aiStatus}
    {aiStatusError}
    onStatusChange={(next) => aiStatus = next}
    onRetry={refreshAiStatus}
    onClose={() => {
      aiSettingsOpen = false;
      // The opener is still inert while the modal `<dialog>` is in the DOM;
      // wait for the `{#if}` block to actually remove it before focusing back.
      void tick().then(() => aiSettingsOpener?.focus());
    }}
  />
{/if}

{#if releaseNotesOpen}
  <ReleaseNotesDialog {projectPath} projectId={project.id} language={activeTab.language} documentName={activeTab.name}
    onCreateDocument={createReleaseDocument} onInsertSections={insertReleaseSections}
    onStatus={(message: string) => status = message} onClose={() => releaseNotesOpen = false} />
{/if}

{#if reviewing}
  <Modal title={`Review changes — ${reviewing.target.name}`} onClose={() => reviewingId = null}>
    <p>The proposal replaces this document's sections, including the diagram source shown below. Nothing changes until you accept.</p>
    {#if reviewing.request.kind === "rework"}
      <p>AI rework of {reworkTargets(reviewing.request, reviewing.baseDocument)} · {reviewing.request.provider}{reviewing.request.model ? ` (${reviewing.request.model})` : ""}</p>
      {#if reviewing.summary}<p class="job-summary">{reviewing.summary}</p>{/if}
    {/if}
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
  <Modal title="Project history from Git" onClose={closeHistory}>
    <p>These are the Git commits that changed this project folder; uncommitted saves are not listed. Restore creates a new working revision; save it to keep it. Your current work remains available through Undo.</p>
    {#if historyError}<p role="alert">{historyError}</p>{/if}
    {#if historyLoading}<p role="status">Loading history…</p>{/if}
    <div class="revision-list">
      {#each historyEntries as entry (entry.commit)}
        <button disabled={historyLoading} onclick={() => previewHistory(entry)}>{entry.commit.slice(0, 8)} · {entry.savedAt} · {entry.summary}</button>
      {/each}
      {#if historyMore}<button disabled={historyLoading} onclick={() => loadHistory(true)}>Load older versions</button>{/if}
    </div>
    {#if historySnapshot}
      <h3>Commit {historyCommit?.commit.slice(0, 8)}: {historySnapshot.name}</h3>
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
      {#if project.revision !== historyBaseRevision}<p role="alert">The project changed during preview. Select the commit again before restoring.</p>{/if}
      <div class="review-actions"><button disabled={historyLoading || project.revision !== historyBaseRevision} onclick={restoreHistory}>Restore this version</button></div>
    {:else if !historyLoading && !historyEntries.length && !historyError}
      <p>No commits contain this project folder yet.</p>
    {/if}
  </Modal>
{/if}

<style>
  .runtime-error { position: fixed; top: 0; left: 0; right: 0; z-index: 10; padding: 16px; background: var(--bg-secondary); color: var(--text-primary); }
  .history-bar { display: flex; align-items: center; gap: 8px; padding: 8px 20px; background: var(--bg-secondary); }
  .history-bar { flex-wrap: wrap; }
  .history-bar .spacer { flex: 1; }
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
  .project-status { padding: 8px 20px; font-size: 12px; overflow-wrap: anywhere; white-space: pre-line; background: var(--bg-secondary); }
  .job-summary { white-space: pre-line; }
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
