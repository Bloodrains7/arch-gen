<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { writeTextFile } from "@tauri-apps/plugin-fs";
  import { marked } from "marked";
  import DOMPurify from "dompurify";
  import { buildHtml, buildMarkdown, exportFileName } from "../export";
  import { localSvgUrl } from "../local-svg";
  import { renderMermaidSvg } from "../mermaid";
  import type { ProjectDocument } from "../project";

  let { document, projectName = "" }: { document: ProjectDocument; projectName?: string } = $props();
  let exportStatus = $state("");
  let exporting = $state(false);

  const today = () => new Date().toLocaleDateString("sv-SE");

  function show(message: string, ms = 5000) {
    exportStatus = message;
    setTimeout(() => { if (exportStatus === message) exportStatus = ""; }, ms);
  }

  // Same profile as the in-app preview: no media, so an exported page makes no requests.
  function renderMarkdown(markdown: string): string {
    return DOMPurify.sanitize(marked.parse(markdown || "", { gfm: true, breaks: true }) as string, {
      USE_PROFILES: { html: true }, FORBID_TAGS: ["img", "video", "audio", "source", "style"], FORBID_ATTR: ["style"],
    });
  }

  async function exportMarkdown() {
    const path = await save({ title: "Export Markdown", defaultPath: exportFileName(document.name, "md"), filters: [{ name: "Markdown", extensions: ["md"] }] });
    if (!path) return;
    try { await writeTextFile(path, buildMarkdown(document, { projectName, date: today() })); show(`Saved to ${path}`); }
    catch (err) { show(`Export failed: ${err}`); }
  }

  // PlantUML diagrams are rendered by the private local renderer (nothing is uploaded);
  // Mermaid is rendered locally in-process (nothing is uploaded either). Anything that
  // cannot be rendered stays as source in the page, and the status says so.
  async function exportHTML() {
    const path = await save({ title: "Export HTML (Word / PDF)", defaultPath: exportFileName(document.name, "html"), filters: [{ name: "HTML", extensions: ["html"] }] });
    if (!path) return;
    exporting = true;
    const images: Record<string, string> = {};
    let asSource = 0;
    try {
      for (const diagram of document.sections.flatMap(section => section.diagrams)) {
        if (!diagram.id || (diagram.format !== "plantuml" && diagram.format !== "mermaid")) { asSource++; continue; }
        try {
          const svg = diagram.format === "plantuml"
            ? await invoke<string>("render_local_diagram", { content: diagram.content })
            : await renderMermaidSvg(diagram.content);
          images[diagram.id] = localSvgUrl(svg);
        } catch { asSource++; }
      }
      await writeTextFile(path, buildHtml(document, { projectName, date: today() }, renderMarkdown, images));
      show(`Saved to ${path}${asSource ? ` — ${asSource} diagram${asSource === 1 ? "" : "s"} included as source (not renderable locally)` : ""}`, 8000);
    } catch (err) { show(`Export failed: ${err}`); }
    finally { exporting = false; }
  }
</script>

<div class="toolbar">
  <div class="toolbar-left">
    <span class="toolbar-label">Template:</span>
    <span class="toolbar-value">{document.template.toUpperCase()}</span>
    <span class="toolbar-separator">|</span>
    <span class="toolbar-label">Sections:</span>
    <span class="toolbar-value">{document.sections.length}</span>
  </div>
  <div class="toolbar-right">
    {#if exportStatus}
      <span class="export-status" role="status">{exportStatus}</span>
    {/if}
    <div class="btn-group">
      <span class="btn-group-label">Export</span>
      <button class="toolbar-btn" onclick={exportMarkdown} title="Markdown with front matter (title, project, template, language, date)">.md</button>
      <button class="toolbar-btn" disabled={exporting} onclick={exportHTML} title="Self-contained page; PlantUML and Mermaid diagrams rendered locally">{exporting ? "Rendering…" : ".html / Word"}</button>
    </div>
    <div class="btn-group">
      <span class="btn-group-label">Diagrams</span>
      <button class="toolbar-btn" disabled title="Export to Microsoft Visio is planned and not implemented yet">Visio</button>
      <button class="toolbar-btn" disabled title="Export to Enterprise Architect is planned and not implemented yet">EA</button>
    </div>
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 32px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .toolbar-left, .toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .toolbar-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .toolbar-value {
    font-size: 12px;
    color: var(--text-primary);
    font-weight: 600;
  }

  .toolbar-separator {
    color: var(--border);
    margin: 0 4px;
  }

  .btn-group {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 8px;
    border-left: 1px solid var(--border);
  }

  .btn-group-label {
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-right: 4px;
  }

  .toolbar-btn {
    padding: 5px 10px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 500;
    transition: all 0.15s;
  }

  .toolbar-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .toolbar-btn:not(:disabled):hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .export-status {
    font-size: 11px;
    color: var(--success);
    font-weight: 500;
  }
</style>
