<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { writeTextFile } from "@tauri-apps/plugin-fs";

  let { selectedTemplate, sections } = $props();
  let exportStatus = $state("");

  async function exportAll(tool: string) {
    exportStatus = `Exporting to ${tool}...`;
    try {
      for (const section of sections) {
        for (const diagram of section.diagrams) {
          await invoke("export_to_tool", { tool, content: diagram.content });
        }
      }
      exportStatus = `Exported to ${tool}`;
      setTimeout(() => (exportStatus = ""), 3000);
    } catch (err) {
      exportStatus = `Export failed: ${err}`;
      setTimeout(() => (exportStatus = ""), 5000);
    }
  }

  function buildMarkdown(): string {
    let md = `# ${selectedTemplate.toUpperCase()} Architecture Documentation\n\n`;
    for (const section of sections) {
      md += `## ${section.title}\n\n`;
      md += `${section.content}\n\n`;
      for (const diagram of section.diagrams) {
        md += `### Diagram (${diagram.diagram_type})\n\n`;
        md += "```" + diagram.format + "\n";
        md += diagram.content + "\n";
        md += "```\n\n";
      }
      md += "---\n\n";
    }
    return md;
  }

  async function exportMarkdown() {
    const md = buildMarkdown();
    const path = await save({
      title: "Export Markdown",
      defaultPath: `architecture-${selectedTemplate}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    await writeTextFile(path, md);
    exportStatus = `Saved to ${path}`;
    setTimeout(() => (exportStatus = ""), 4000);
  }

  async function exportHTML() {
    let html = `<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<title>${selectedTemplate.toUpperCase()} Architecture Documentation</title>
<style>
  body { font-family: 'Segoe UI', Calibri, Arial, sans-serif; max-width: 900px; margin: 40px auto; padding: 0 20px; color: #1a1a1a; line-height: 1.6; }
  h1 { color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; }
  h2 { color: #2c3e50; border-bottom: 1px solid #bdc3c7; padding-bottom: 6px; margin-top: 30px; }
  h3 { color: #34495e; }
  table { border-collapse: collapse; width: 100%; margin: 12px 0; }
  th, td { border: 1px solid #bdc3c7; padding: 8px 12px; text-align: left; }
  th { background: #ecf0f1; font-weight: 600; }
  pre { background: #f8f9fa; border: 1px solid #dee2e6; border-radius: 4px; padding: 12px; overflow-x: auto; font-size: 13px; }
  code { background: #f8f9fa; padding: 2px 5px; border-radius: 3px; font-size: 13px; }
  ul, ol { margin: 8px 0; }
  li { margin: 4px 0; }
  blockquote { border-left: 4px solid #3498db; margin: 12px 0; padding: 8px 16px; background: #f0f7ff; }
  hr { border: none; border-top: 1px solid #dee2e6; margin: 24px 0; }
  .diagram-block { background: #f8f9fa; border: 1px solid #dee2e6; border-radius: 6px; padding: 16px; margin: 12px 0; }
  .diagram-label { font-size: 11px; text-transform: uppercase; letter-spacing: 1px; color: #7f8c8d; margin-bottom: 8px; }
  @media print { body { margin: 20px; } h1 { page-break-before: avoid; } h2 { page-break-after: avoid; } }
</style>
</head><body>`;

    html += `<h1>${selectedTemplate.toUpperCase()} Architecture Documentation</h1>\n`;
    html += `<p><em>Generated: ${new Date().toLocaleDateString()}</em></p>\n`;

    for (const section of sections) {
      html += `<h2>${escapeHtml(section.title)}</h2>\n`;
      html += `<div>${markdownToBasicHtml(section.content)}</div>\n`;
      for (const diagram of section.diagrams) {
        html += `<div class="diagram-block">`;
        html += `<div class="diagram-label">${escapeHtml(diagram.diagram_type)} (${escapeHtml(diagram.format)})</div>`;
        html += `<pre><code>${escapeHtml(diagram.content)}</code></pre>`;
        html += `</div>\n`;
      }
    }

    html += `</body></html>`;

    const path = await save({
      title: "Export HTML (Word / PDF)",
      defaultPath: `architecture-${selectedTemplate}.html`,
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (!path) return;
    await writeTextFile(path, html);
    exportStatus = `Saved to ${path}`;
    setTimeout(() => (exportStatus = ""), 4000);
  }

  function escapeHtml(text: string): string {
    return text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function markdownToBasicHtml(md: string): string {
    if (!md) return "";
    let html = escapeHtml(md);
    // Headers
    html = html.replace(/^### (.+)$/gm, "<h3>$1</h3>");
    html = html.replace(/^## (.+)$/gm, "<h4>$1</h4>");
    // Bold
    html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
    // Italic
    html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
    // Checkboxes
    html = html.replace(/- \[ \] /g, "&#9744; ");
    html = html.replace(/- \[x\] /g, "&#9745; ");
    // Bullet points
    html = html.replace(/^- (.+)$/gm, "<li>$1</li>");
    html = html.replace(/(<li>.*<\/li>\n?)+/g, "<ul>$&</ul>");
    // Numbered lists
    html = html.replace(/^\d+\. (.+)$/gm, "<li>$1</li>");
    // Code blocks
    html = html.replace(/```[\w]*\n([\s\S]*?)```/g, "<pre><code>$1</code></pre>");
    // Inline code
    html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
    // Blockquotes
    html = html.replace(/^&gt; (.+)$/gm, "<blockquote>$1</blockquote>");
    // Tables (basic)
    html = html.replace(/\|(.+)\|\n\|[-| ]+\|\n((?:\|.+\|\n?)*)/gm, (_, header, rows) => {
      const headers = header.split("|").map((h: string) => h.trim()).filter(Boolean);
      let table = "<table><thead><tr>";
      headers.forEach((h: string) => { table += `<th>${h}</th>`; });
      table += "</tr></thead><tbody>";
      rows.trim().split("\n").forEach((row: string) => {
        const cells = row.split("|").map((c: string) => c.trim()).filter(Boolean);
        table += "<tr>";
        cells.forEach((c: string) => { table += `<td>${c}</td>`; });
        table += "</tr>";
      });
      table += "</tbody></table>";
      return table;
    });
    // HR
    html = html.replace(/^---$/gm, "<hr>");
    // Paragraphs
    html = html.replace(/\n\n/g, "</p><p>");
    html = html.replace(/\n/g, "<br>");
    return `<p>${html}</p>`;
  }
</script>

<div class="toolbar">
  <div class="toolbar-left">
    <span class="toolbar-label">Template:</span>
    <span class="toolbar-value">{selectedTemplate.toUpperCase()}</span>
    <span class="toolbar-separator">|</span>
    <span class="toolbar-label">Sections:</span>
    <span class="toolbar-value">{sections.length}</span>
  </div>
  <div class="toolbar-right">
    {#if exportStatus}
      <span class="export-status">{exportStatus}</span>
    {/if}
    <div class="btn-group">
      <span class="btn-group-label">Export</span>
      <button class="toolbar-btn" onclick={exportMarkdown}>.md</button>
      <button class="toolbar-btn" onclick={exportHTML}>.html / Word</button>
    </div>
    <div class="btn-group">
      <span class="btn-group-label">Diagrams</span>
      <button class="toolbar-btn" onclick={() => exportAll("visio")}>Visio</button>
      <button class="toolbar-btn" onclick={() => exportAll("ea")}>EA</button>
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

  .toolbar-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .export-status {
    font-size: 11px;
    color: var(--success);
    font-weight: 500;
  }
</style>
