<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { confirm, open, save } from "@tauri-apps/plugin-dialog";
  import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
  import Modal from "./Modal.svelte";
  import MarkdownPreview from "./MarkdownPreview.svelte";
  import { BUILTIN_TEMPLATES, TEMPLATE_HELP, markdownToSections, parseTemplate, renderReleaseNotes } from "../release-notes";
  import type { GitRange, ReleaseTemplate } from "../release-notes";
  import { defaultRange, errorText } from "../git";
  import type { GitRefs, TemplateFile } from "../git";
  import type { Section } from "../project";

  let {
    projectPath = null as string | null,
    projectId = "",
    language = "en",
    documentName = "",
    onCreateDocument = (_name: string, _sections: Section[]) => {},
    onInsertSections = (_sections: Section[]) => {},
    onStatus = (_message: string) => {},
    onClose = () => {},
  } = $props();

  // sv-SE formats as YYYY-MM-DD in local time.
  const today = new Date().toLocaleDateString("sv-SE");
  let repository = $state("");
  let refs = $state<GitRefs | null>(null);
  let refsError = $state("");
  let from = $state("");
  let to = $state("HEAD");
  let subpath = $state("");
  let includeMerges = $state(false);
  let version = $state("Unreleased");
  let date = $state(today);
  let range = $state<GitRange | null>(null);
  // The inputs the loaded commits belong to: notes are never written for another range.
  let loadedFor = $state("");
  let rangeKey = $derived(JSON.stringify([repository.trim(), from.trim(), to.trim(), subpath.trim(), includeMerges]));
  let stale = $derived(!!range && loadedFor !== rangeKey);
  let loading = $state(false);
  let rangeError = $state("");
  let projectTemplates = $state<TemplateFile[]>([]);
  let templateId = $state("builtin:keep-a-changelog");
  let source = $state("");
  let fileError = $state("");
  let showSource = $state(false);
  let showHelp = $state(false);
  let request = 0;

  let options = $derived([
    ...BUILTIN_TEMPLATES.map(t => ({ id: t.id, label: `${safeName(t.source)} (built-in)`, source: t.source })),
    ...projectTemplates.filter(t => !t.error).map(t => ({ id: `project:${t.file}`, label: `${safeName(t.content, t.file)} (project)`, source: t.content })),
  ]);
  let parsed = $derived.by((): { template: ReleaseTemplate | null; error: string } => {
    try { return { template: parseTemplate(source, "Custom template"), error: "" }; }
    catch (e) { return { template: null, error: errorText(e) }; }
  });
  let result = $derived.by(() => {
    if (!range || !parsed.template) return null;
    try {
      return renderReleaseNotes(range, parsed.template, { version, date, from, to, repository: range.root.split(/[\\/]/).filter(Boolean).pop() ?? "" });
    } catch (e) { return { error: errorText(e) }; }
  });
  let rendered = $derived(result && "markdown" in result ? result : null);
  let hiddenTypes = $derived(rendered ? Object.entries(rendered.hidden.reduce<Record<string, number>>((all, item) => ({ ...all, [item.type]: (all[item.type] ?? 0) + 1 }), {})).map(([type, n]) => `${type} ×${n}`).join(", ") : "");

  function safeName(text: string, fallback = "Template") {
    try { return parseTemplate(text, fallback).name; } catch { return fallback; }
  }

  function selectTemplate(id: string) {
    const option = options.find(o => o.id === id);
    if (!option) return;
    templateId = id; source = option.source;
  }

  async function loadRefs() {
    const path = repository.trim();
    const token = ++request;
    refs = null; range = null; refsError = ""; rangeError = "";
    if (!path) return;
    try {
      const next = await invoke<GitRefs>("git_list_refs", { path });
      if (token !== request) return;
      refs = next;
      const initial = defaultRange(next);
      from = initial.from; to = initial.to; version = initial.version;
      if (next.head) await loadCommits(token);
      else rangeError = "This repository has no commits yet.";
    } catch (e) { if (token === request) refsError = errorText(e); }
  }

  async function loadCommits(token = ++request) {
    if (!refs) return;
    loading = true; rangeError = "";
    const key = rangeKey;
    try {
      const next = await invoke<GitRange>("git_log_range", { path: repository.trim(), from: from.trim() || null, to: to.trim() || "HEAD", subpath: subpath.trim() || null, includeMerges });
      if (token === request) { range = next; loadedFor = key; }
    } catch (e) { if (token === request) { range = null; rangeError = errorText(e); } }
    finally { if (token === request) loading = false; }
  }

  async function chooseRepository() {
    const path = await open({ title: "Choose a folder inside the Git repository", directory: true, multiple: false });
    if (typeof path === "string") { repository = path; await loadRefs(); }
  }

  async function loadProjectTemplates() {
    if (!projectPath) return;
    try {
      projectTemplates = await invoke<TemplateFile[]>("list_release_templates", { path: projectPath, projectId });
      const broken = projectTemplates.filter(t => t.error);
      fileError = broken.length ? `Skipped project templates: ${broken.map(t => `${t.file} (${t.error})`).join(", ")}` : "";
    } catch (e) { fileError = `Project templates could not be read: ${errorText(e)}`; }
  }

  async function loadTemplateFile() {
    const path = await open({ title: "Open a release-note template", multiple: false, filters: [{ name: "Markdown template", extensions: ["md", "markdown", "txt"] }] });
    if (typeof path !== "string") return;
    try { source = await readTextFile(path); templateId = "file"; fileError = ""; }
    catch (e) { fileError = `Could not read ${path}: ${errorText(e)}`; }
  }

  async function saveTemplateToProject() {
    if (!projectPath || !parsed.template) return;
    const name = prompt("Template name (saved as templates/release-notes/<name>.md in the project folder):", parsed.template.name);
    if (!name) return;
    try {
      let file: string;
      try { file = await invoke<string>("save_release_template", { path: projectPath, projectId, name, content: source, overwrite: false }); }
      catch (e) {
        if (!errorText(e).includes("already exists") || !(await confirm(`${errorText(e)}\n\nOverwrite it?`, { title: "Template exists", kind: "warning" }))) throw e;
        file = await invoke<string>("save_release_template", { path: projectPath, projectId, name, content: source, overwrite: true });
      }
      await loadProjectTemplates();
      templateId = `project:${file}`;
      onStatus(`Saved release-note template templates/release-notes/${file}. Commit it to share it with the team.`);
    } catch (e) { fileError = `Template was not saved: ${errorText(e)}`; }
  }

  function sections() {
    return rendered && !stale ? markdownToSections(rendered.markdown, `Release ${version}`) : null;
  }

  function createDocument() {
    const doc = sections();
    if (!doc) return;
    onCreateDocument(doc.title ?? `Release notes ${version}`, doc.sections);
    onClose();
  }

  function insertIntoDocument() {
    const doc = sections();
    if (!doc) return;
    onInsertSections(doc.sections);
    onClose();
  }

  async function saveFile() {
    if (!rendered || stale) return;
    const path = await save({ title: "Save release notes", defaultPath: `release-notes-${version.replace(/[^\w.-]+/g, "-")}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!path) return;
    try { await writeTextFile(path, rendered.markdown); onStatus(`Saved release notes to ${path}`); }
    catch (e) { onStatus(`Saving release notes failed: ${errorText(e)}`); }
  }

  async function copy() {
    if (!rendered || stale) return;
    try { await navigator.clipboard.writeText(rendered.markdown); onStatus("Release notes copied as Markdown."); }
    catch (e) { onStatus(`Copy failed: ${errorText(e)}`); }
  }

  onMount(() => {
    repository = projectPath ?? "";
    selectTemplate(language === "sk" ? "builtin:customer-sk" : "builtin:keep-a-changelog");
    void loadProjectTemplates();
    void loadRefs();
  });
</script>

<Modal title="Release notes from Git" onClose={() => onClose()}>
  <p class="intro">Reads commits of a Git range and writes release notes from a template. Nothing is sent anywhere and the repository is not changed. Commits following <a href="https://www.conventionalcommits.org/" target="_blank" rel="noopener">Conventional Commits</a> (feat:, fix:, …) are sorted into the template's groups.</p>

  <fieldset>
    <legend>Repository and range</legend>
    <div class="row">
      <label class="grow">Repository folder <input aria-label="Repository folder" bind:value={repository} onchange={loadRefs} placeholder="Folder inside a Git repository" /></label>
      <button onclick={chooseRepository}>Choose folder…</button>
      {#if projectPath && repository !== projectPath}<button onclick={() => { repository = projectPath!; void loadRefs(); }}>Use project folder</button>{/if}
    </div>
    {#if refsError}<p class="warn" role="alert">{refsError}</p>{/if}
    {#if refs}<p class="hint">Repository {refs.root}{refs.branch ? ` · on ${refs.branch}` : ""} · {refs.refs.filter(r => r.kind === "tag").length} tags</p>{/if}
    <datalist id="release-refs">
      <option value="HEAD"></option>
      {#each refs?.refs ?? [] as ref (`${ref.kind}:${ref.name}`)}<option value={ref.name}>{ref.kind} · {ref.date.slice(0, 10)}</option>{/each}
    </datalist>
    <div class="row">
      <label>From (excluded) <input aria-label="From revision" list="release-refs" bind:value={from} placeholder="beginning of history" /></label>
      <label>To (included) <input aria-label="To revision" list="release-refs" bind:value={to} /></label>
      <label class="grow">Only folder <input aria-label="Only commits touching folder" bind:value={subpath} placeholder="whole repository, or e.g. services/billing" /></label>
      <label class="check"><input type="checkbox" bind:checked={includeMerges} /> Merge commits</label>
      <button disabled={!refs || loading} onclick={() => loadCommits()}>{loading ? "Loading…" : "Load commits"}</button>
    </div>
    {#if rangeError}<p class="warn" role="alert">{rangeError}</p>{/if}
  </fieldset>

  <fieldset>
    <legend>Template</legend>
    <div class="row">
      <label class="grow">Template
        <select aria-label="Release-note template" value={templateId} onchange={(e) => selectTemplate(e.currentTarget.value)}>
          {#each options as option (option.id)}<option value={option.id}>{option.label}</option>{/each}
          {#if templateId === "file" || templateId === "edited"}<option value={templateId}>{templateId === "file" ? "From file" : "Edited"}: {parsed.template?.name ?? "invalid template"}</option>{/if}
        </select>
      </label>
      <label>Version <input aria-label="Version" bind:value={version} /></label>
      <label>Date <input aria-label="Release date" bind:value={date} /></label>
    </div>
    <div class="row">
      <button onclick={() => showSource = !showSource}>{showSource ? "Hide template source" : "Edit template"}</button>
      <button onclick={loadTemplateFile}>Load template file…</button>
      {#if projectPath}<button disabled={!parsed.template} onclick={saveTemplateToProject}>Save template to project…</button>{/if}
      <button onclick={() => showHelp = !showHelp}>Template syntax</button>
    </div>
    {#if parsed.template?.description}<p class="hint">{parsed.template.description}</p>{/if}
    {#if showHelp}<pre class="help">{TEMPLATE_HELP}</pre>{/if}
    {#if showSource}
      <textarea aria-label="Template source" class="source" rows="14" value={source} oninput={(e) => { source = e.currentTarget.value; templateId = "edited"; }}></textarea>
    {/if}
    {#if parsed.error}<p class="warn" role="alert">{parsed.error}</p>{/if}
    {#if fileError}<p class="warn">{fileError}</p>{/if}
  </fieldset>

  {#if result && "error" in result}<p class="warn" role="alert">{result.error}</p>{/if}
  {#if rendered && range}
    <p class="hint" role="status">{range.commits.length} commit{range.commits.length === 1 ? "" : "s"}{range.truncated ? " (only the newest 2000)" : ""} · {rendered.included.length} in the notes{rendered.hidden.length ? ` · ${rendered.hidden.length} left out by the template (${hiddenTypes})` : ""}</p>
    {#if rendered.hidden.length}
      <details><summary>Left out</summary>
        <ul class="hidden-list">{#each rendered.hidden as item (item.hash)}<li><code>{item.shortHash}</code> {item.subject}</li>{/each}</ul>
      </details>
    {/if}
    {#if stale}<p class="warn" role="alert">The repository or range changed. Load commits again to update the notes.</p>{/if}
    <div class="preview" class:stale><MarkdownPreview content={rendered.markdown} /></div>
    <div class="actions">
      <button disabled={stale} onclick={createDocument}>Create document</button>
      <button disabled={stale} onclick={insertIntoDocument} title={`Adds the release at the top of ${documentName}`}>Insert at top of "{documentName}"</button>
      <button disabled={stale} onclick={saveFile}>Save as Markdown file…</button>
      <button disabled={stale} onclick={copy}>Copy Markdown</button>
    </div>
    <p class="hint">Tip: after inserting, tick the sections and use AI Rework to rewrite commit messages for your readers.</p>
  {/if}
</Modal>

<style>
  .intro, .hint { font-size: 12px; color: var(--text-secondary); margin: 6px 0; }
  .intro a { color: var(--accent); }
  fieldset { border: 1px solid var(--border); border-radius: 6px; padding: 10px 14px; margin: 12px 0; }
  legend { font-size: 12px; color: var(--text-muted); padding: 0 6px; text-transform: uppercase; letter-spacing: 0.5px; }
  .row { display: flex; flex-wrap: wrap; gap: 10px; align-items: flex-end; margin: 6px 0; }
  label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-secondary); }
  label.check { flex-direction: row; align-items: center; padding-bottom: 8px; }
  .grow { flex: 1; min-width: 220px; }
  input, select, textarea { padding: 6px 8px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; font-family: inherit; font-size: 13px; }
  .source, .help { width: 100%; font-family: monospace; font-size: 12px; }
  .help { white-space: pre-wrap; padding: 8px; background: var(--bg-tertiary); border-radius: 4px; margin: 6px 0; }
  button { padding: 7px 12px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; }
  button:disabled { opacity: 0.5; cursor: default; }
  .warn { color: var(--warning); font-size: 13px; margin: 6px 0; }
  .preview { margin: 10px 0; }
  .preview.stale { opacity: 0.5; }
  .hidden-list { font-size: 12px; margin: 6px 0 6px 18px; }
  .actions { display: flex; flex-wrap: wrap; gap: 10px; position: sticky; bottom: -20px; padding: 12px 0; background: var(--bg-secondary); }
</style>
